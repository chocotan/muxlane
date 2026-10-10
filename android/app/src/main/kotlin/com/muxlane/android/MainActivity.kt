package com.muxlane.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels

class MainActivity : ComponentActivity() {
    private val model by viewModels<MuxlaneViewModel>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.light(0xFFF6F8F7.toInt(), 0xFF101413.toInt()),
            navigationBarStyle = SystemBarStyle.light(0xFFF6F8F7.toInt(), 0xFF101413.toInt()),
        )
        if (android.os.Build.VERSION.SDK_INT >= 29) {
            window.isNavigationBarContrastEnforced = false
        }
        if (savedInstanceState == null) handlePairLink(intent)
        handleNotification(intent)
        setContent {
            MuxlaneTheme {
                MuxlaneRoot(model)
            }
        }
    }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handlePairLink(intent)
        handleNotification(intent)
    }

    private fun handlePairLink(intent: android.content.Intent?) {
        val (relay, hostId) = parsePairLink(intent?.dataString ?: return) ?: return
        model.beginAddMachine(
            relayUrl = relay,
            hostId = hostId,
        )
    }

    private fun handleNotification(intent: android.content.Intent?) {
        val host = intent?.getStringExtra(RelayService.EXTRA_HOST)
        val agent = intent?.getStringExtra(RelayService.EXTRA_AGENT)
        if (!host.isNullOrBlank() && !agent.isNullOrBlank()) model.openNotification(host, agent)
    }
}
