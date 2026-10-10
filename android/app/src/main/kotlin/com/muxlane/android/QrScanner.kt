package com.muxlane.android

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.google.zxing.BarcodeFormat
import com.journeyapps.barcodescanner.BarcodeCallback
import com.journeyapps.barcodescanner.BarcodeResult
import com.journeyapps.barcodescanner.BarcodeView
import com.journeyapps.barcodescanner.CameraPreview
import com.journeyapps.barcodescanner.DefaultDecoderFactory

/** Camera and QR decoding run inside the app, including on devices without Play services. */
class QrScanActivity : ComponentActivity() {
    private lateinit var preview: BarcodeView
    private var cameraGranted by mutableStateOf(false)
    private var cameraReady by mutableStateOf(false)
    private var cameraError by mutableStateOf(false)
    private var invalidCode by mutableStateOf(false)
    private var torchOn by mutableStateOf(false)
    private var resumed = false
    private val cameraPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        cameraGranted = granted
        if (granted && resumed) startCamera()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        preview = BarcodeView(this).apply {
            decoderFactory = DefaultDecoderFactory(listOf(BarcodeFormat.QR_CODE))
            addStateListener(object : CameraPreview.StateListener {
                override fun previewSized() = Unit
                override fun previewStarted() { cameraReady = true }
                override fun previewStopped() { cameraReady = false }
                override fun cameraClosed() = Unit
                override fun cameraError(error: Exception) {
                    cameraReady = false
                    cameraError = true
                    preview.setTorch(false)
                    torchOn = false
                }
            })
            decodeContinuous(object : BarcodeCallback {
                override fun barcodeResult(result: BarcodeResult) {
                    if (!resumed || isFinishing) return
                    if (parsePairQr(result.text) == null) {
                        invalidCode = true
                        return
                    }
                    setResult(RESULT_OK, Intent().putExtra(EXTRA_TEXT, result.text))
                    pause()
                    finish()
                }
            })
        }
        cameraGranted = checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
        setContent {
            MuxlaneTheme {
                Surface(Modifier.fillMaxSize(), shape = Square) {
                    Column(Modifier.fillMaxSize().safeDrawingPadding()) {
                        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                            Text(stringResource(R.string.scan_qr), modifier = Modifier.weight(1f), style = MaterialTheme.typography.titleMedium)
                            TextButton(onClick = { finish() }, shape = Square, modifier = Modifier.heightIn(min = 48.dp)) {
                                Text(stringResource(R.string.cancel))
                            }
                        }
                        Box(Modifier.fillMaxWidth().weight(1f).background(Color.Black)) {
                            if (cameraGranted && !cameraError) {
                                AndroidView(factory = { preview }, modifier = Modifier.fillMaxSize())
                                if (cameraReady) {
                                    Box(Modifier.align(Alignment.Center).fillMaxWidth(0.7f).aspectRatio(1f).border(2.dp, Color.White, Square))
                                } else {
                                    Text(stringResource(R.string.scan_qr_loading), color = Color.White, modifier = Modifier.align(Alignment.Center))
                                }
                            } else {
                                Column(
                                    Modifier.align(Alignment.Center).fillMaxWidth().verticalScroll(rememberScrollState()).padding(24.dp),
                                    horizontalAlignment = Alignment.CenterHorizontally,
                                    verticalArrangement = Arrangement.spacedBy(16.dp),
                                ) {
                                    Text(stringResource(if (cameraGranted) R.string.scan_qr_camera_error else R.string.scan_qr_permission), color = Color.White)
                                    Button(
                                        onClick = { if (cameraGranted) startCamera() else cameraPermission.launch(Manifest.permission.CAMERA) },
                                        shape = Square,
                                        modifier = Modifier.heightIn(min = 48.dp),
                                    ) { Text(stringResource(if (cameraGranted) R.string.retry else R.string.scan_qr_grant)) }
                                    if (!cameraGranted) {
                                        OutlinedButton(
                                            onClick = { startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName"))) },
                                            shape = Square,
                                            modifier = Modifier.heightIn(min = 48.dp),
                                        ) { Text(stringResource(R.string.scan_qr_settings), color = Color.White) }
                                    }
                                }
                            }
                        }
                        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
                            if (invalidCode) Text(stringResource(R.string.scan_qr_invalid), color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(vertical = 16.dp))
                            if (packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_FLASH)) {
                                TextButton(
                                    onClick = { torchOn = !torchOn; preview.setTorch(torchOn) },
                                    enabled = cameraGranted && cameraReady && !cameraError,
                                    shape = Square,
                                    modifier = Modifier.heightIn(min = 48.dp),
                                ) { Text(stringResource(if (torchOn) R.string.scan_qr_torch_off else R.string.scan_qr_torch_on)) }
                            }
                        }
                    }
                }
            }
        }
        if (!cameraGranted && savedInstanceState == null) cameraPermission.launch(Manifest.permission.CAMERA)
    }

    override fun onResume() {
        super.onResume()
        resumed = true
        cameraGranted = checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
        if (cameraGranted) startCamera()
    }

    override fun onPause() {
        resumed = false
        preview.setTorch(false)
        preview.pause()
        cameraReady = false
        torchOn = false
        super.onPause()
    }

    private fun startCamera() {
        cameraError = false
        if (!packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY)) {
            cameraError = true
            return
        }
        preview.resume()
    }

    companion object {
        const val EXTRA_TEXT = "qr_text"
        fun intent(context: Context) = Intent(context, QrScanActivity::class.java)
    }
}
