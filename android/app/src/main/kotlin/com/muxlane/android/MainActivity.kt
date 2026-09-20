package com.muxlane.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels

class MainActivity : ComponentActivity() {
    private val model by viewModels<MuxlaneViewModel>()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        handleAgentExtra(intent?.getStringExtra(RelayService.EXTRA_AGENT))
        setContent {
            MuxlaneTheme {
                MuxlaneRoot(model)
            }
        }
    }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        handleAgentExtra(intent.getStringExtra(RelayService.EXTRA_AGENT))
    }

    private fun handleAgentExtra(agent: String?) {
        if (!agent.isNullOrBlank()) {
            model.openAgent(agent)
            model.markSeen(agent)
        }
    }
}
