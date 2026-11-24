package muxlane.crypto

import kotlinx.serialization.Serializable
import java.math.BigInteger
import java.security.KeyFactory
import java.security.KeyPair
import java.security.KeyPairGenerator
import java.security.SecureRandom
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.ECPoint
import java.security.spec.ECPublicKeySpec
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.KeyAgreement
import javax.crypto.Mac
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

@Serializable
data class E2eeEnvelope(
    val e2ee: Int = 1,
    val s: Long,
    val p: String,
)

@Serializable
data class E2eeUpgradeParams(
    val client_pubkey_b64: String,
    val client_nonce_b64: String,
)

@Serializable
data class E2eeUpgradeResult(
    val server_pubkey_b64: String,
    val server_nonce_b64: String,
    val cipher: String,
)

class E2eeKeyPair private constructor(
    private val keyPair: KeyPair,
    val publicSec1: ByteArray,
) {
    val ecPublicKey: ECPublicKey
        get() = keyPair.public as ECPublicKey

    fun diffieHellman(peerSec1: ByteArray): ByteArray {
        require(peerSec1.size == 65 && peerSec1[0] == 0x04.toByte()) {
            "Expected 65-byte uncompressed P-256 public key"
        }
        val x = BigInteger(1, peerSec1.copyOfRange(1, 33))
        val y = BigInteger(1, peerSec1.copyOfRange(33, 65))
        val spec = ECPublicKeySpec(ECPoint(x, y), ecPublicKey.params)
        val peerPub = KeyFactory.getInstance("EC").generatePublic(spec)
        val agreement = KeyAgreement.getInstance("ECDH")
        agreement.init(keyPair.private)
        agreement.doPhase(peerPub, true)
        val secret = agreement.generateSecret()
        return if (secret.size == 32) {
            secret
        } else {
            toUnsigned32(BigInteger(1, secret))
        }
    }

    companion object {
        private val rng = SecureRandom()

        fun generate(): E2eeKeyPair {
            val gen = KeyPairGenerator.getInstance("EC")
            gen.initialize(ECGenParameterSpec("secp256r1"), rng)
            val kp = gen.generateKeyPair()
            val pub = kp.public as ECPublicKey
            val sec1 = ByteArray(65)
            sec1[0] = 0x04
            val x = toUnsigned32(pub.w.affineX)
            val y = toUnsigned32(pub.w.affineY)
            System.arraycopy(x, 0, sec1, 1, 32)
            System.arraycopy(y, 0, sec1, 33, 32)
            return E2eeKeyPair(kp, sec1)
        }

        fun randomNonce16(): ByteArray = ByteArray(16).also(rng::nextBytes)

        private fun toUnsigned32(value: BigInteger): ByteArray {
            val raw = value.toByteArray()
            val out = ByteArray(32)
            val srcOffset = if (raw.size > 32 && raw[0] == 0.toByte()) raw.size - 32 else 0
            val copyLen = minOf(32, raw.size - srcOffset)
            System.arraycopy(raw, srcOffset, out, 32 - copyLen, copyLen)
            return out
        }
    }
}

class E2eeSession private constructor(
    private val txKey: SecretKeySpec,
    private val rxKey: SecretKeySpec,
    private val txDir: Byte,
    private val rxDir: Byte,
) {
    private var txSeq = 0L
    private var rxSeq = 0L

    @Synchronized
    fun seal(plaintext: ByteArray): E2eeEnvelope {
        txSeq += 1
        val nonce = buildNonce(txDir, txSeq)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, txKey, GCMParameterSpec(128, nonce))
        cipher.updateAAD(nonce)
        val ct = cipher.doFinal(plaintext)
        return E2eeEnvelope(
            e2ee = 1,
            s = txSeq,
            p = Base64.getEncoder().encodeToString(ct),
        )
    }

    @Synchronized
    fun open(env: E2eeEnvelope): ByteArray {
        require(env.e2ee == 1) { "Unsupported e2ee version: ${env.e2ee}" }
        require(env.s > rxSeq) { "Replayed or out-of-order e2ee frame" }
        val nonce = buildNonce(rxDir, env.s)
        val ct = Base64.getDecoder().decode(env.p)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, rxKey, GCMParameterSpec(128, nonce))
        cipher.updateAAD(nonce)
        val pt = cipher.doFinal(ct)
        rxSeq = env.s
        return pt
    }

    companion object {
        private val INFO_C2S = "muxlane-e2ee-c2s-v1".toByteArray(Charsets.UTF_8)
        private val INFO_S2C = "muxlane-e2ee-s2c-v1".toByteArray(Charsets.UTF_8)

        fun deriveClient(
            localKeys: E2eeKeyPair,
            serverPubSec1: ByteArray,
            pairingToken: String,
            clientNonce: ByteArray,
            serverNonce: ByteArray,
        ): E2eeSession = derive(true, localKeys, serverPubSec1, pairingToken, clientNonce, serverNonce)

        fun deriveServer(
            localKeys: E2eeKeyPair,
            clientPubSec1: ByteArray,
            pairingToken: String,
            clientNonce: ByteArray,
            serverNonce: ByteArray,
        ): E2eeSession = derive(false, localKeys, clientPubSec1, pairingToken, clientNonce, serverNonce)

        private fun derive(
            isClient: Boolean,
            localKeys: E2eeKeyPair,
            peerPubSec1: ByteArray,
            pairingToken: String,
            clientNonce: ByteArray,
            serverNonce: ByteArray,
        ): E2eeSession {
            val shared = localKeys.diffieHellman(peerPubSec1)
            val tokenBytes = pairingToken.toByteArray(Charsets.UTF_8)
            val salt = ByteArray(tokenBytes.size + clientNonce.size + serverNonce.size)
            System.arraycopy(tokenBytes, 0, salt, 0, tokenBytes.size)
            System.arraycopy(clientNonce, 0, salt, tokenBytes.size, clientNonce.size)
            System.arraycopy(serverNonce, 0, salt, tokenBytes.size + clientNonce.size, serverNonce.size)

            val prk = hmacSha256(if (salt.isEmpty()) ByteArray(32) else salt, shared)
            val c2s = hkdfExpand32(prk, INFO_C2S)
            val s2c = hkdfExpand32(prk, INFO_S2C)

            return if (isClient) {
                E2eeSession(SecretKeySpec(c2s, "AES"), SecretKeySpec(s2c, "AES"), 0x01, 0x02)
            } else {
                E2eeSession(SecretKeySpec(s2c, "AES"), SecretKeySpec(c2s, "AES"), 0x02, 0x01)
            }
        }

        private fun hmacSha256(key: ByteArray, data: ByteArray): ByteArray {
            val mac = Mac.getInstance("HmacSHA256")
            mac.init(SecretKeySpec(key, "HmacSHA256"))
            return mac.doFinal(data)
        }

        private fun hkdfExpand32(prk: ByteArray, info: ByteArray): ByteArray {
            val mac = Mac.getInstance("HmacSHA256")
            mac.init(SecretKeySpec(prk, "HmacSHA256"))
            mac.update(info)
            mac.update(0x01.toByte())
            return mac.doFinal()
        }

        private fun buildNonce(dir: Byte, seq: Long): ByteArray {
            val out = ByteArray(12)
            out[0] = dir
            for (i in 0 until 8) {
                out[11 - i] = ((seq ushr (i * 8)) and 0xff).toByte()
            }
            return out
        }
    }
}
