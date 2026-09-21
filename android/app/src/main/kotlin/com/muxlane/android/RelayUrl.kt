package com.muxlane.android

import java.net.URI

fun normalizeRelayUrl(raw: String): String {
    val value = raw.trim().trimEnd('/')
    require(value.isNotEmpty()) { "请填写中继地址" }
    val uri = runCatching { URI(value) }.getOrElse { error("中继地址格式不正确") }
    val scheme = uri.scheme?.lowercase()
    require(scheme == "ws" || scheme == "wss") { "中继地址必须以 wss:// 或 ws:// 开头" }
    val host = uri.host?.lowercase() ?: error("中继地址缺少主机名")
    require(uri.userInfo == null && uri.fragment == null && uri.query == null) { "中继地址不能包含账号、查询参数或片段" }
    require(scheme == "wss" || isLocalHost(host)) { "公网中继必须使用 wss:// 加密连接" }
    return value
}

private fun isLocalHost(host: String): Boolean {
    if (host == "localhost" || host.endsWith(".local") || '.' !in host && ':' !in host) return true
    val ipv4 = host.split('.').mapNotNull(String::toIntOrNull)
    if (ipv4.size == 4 && ipv4.all { it in 0..255 }) {
        return ipv4[0] == 10 ||
            ipv4[0] == 127 ||
            ipv4[0] == 169 && ipv4[1] == 254 ||
            ipv4[0] == 192 && ipv4[1] == 168 ||
            ipv4[0] == 172 && ipv4[1] in 16..31
    }
    return host == "::1" || host.startsWith("fc") || host.startsWith("fd") || host.startsWith("fe80:")
}
