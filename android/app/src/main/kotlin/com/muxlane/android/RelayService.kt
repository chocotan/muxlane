package com.muxlane.android

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import muxlane.protocol.AgentInstance
import muxlane.protocol.AgentStatus
import muxlane.protocol.AgentStatusEvent
import muxlane.protocol.isAlert

class RelayService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        ensureChannel(this)
        val notification = NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .setContentTitle("Muxlane")
            .setContentText("连着那台机器")
            .setOngoing(true)
            .build()
        startForeground(1, notification)
        return START_STICKY
    }

    companion object {
        const val CHANNEL = "muxlane-status"
        const val EXTRA_AGENT = "agent"

        fun ensureChannel(context: Context) {
            if (Build.VERSION.SDK_INT >= 26) {
                val manager = context.getSystemService(NotificationManager::class.java)
                manager.createNotificationChannel(
                    NotificationChannel(CHANNEL, "Muxlane 会话", NotificationManager.IMPORTANCE_DEFAULT),
                )
            }
        }

        fun notifyStatus(context: Context, agent: AgentInstance?, event: AgentStatusEvent) {
            if (!event.to.isAlert()) return
            ensureChannel(context)
            val title = when (event.to) {
                AgentStatus.BLOCKED -> "等待输入"
                AgentStatus.DONE -> "任务完成"
                AgentStatus.FAILED -> "任务失败"
                else -> event.to.name
            }
            val body = event.message ?: agent?.title ?: event.agent
            val open = Intent(context, MainActivity::class.java).putExtra(EXTRA_AGENT, event.agent)
            val pending = PendingIntent.getActivity(
                context,
                event.agent.hashCode(),
                open,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val notification = NotificationCompat.Builder(context, CHANNEL)
                .setSmallIcon(android.R.drawable.ic_dialog_info)
                .setContentTitle(title)
                .setContentText(body)
                .setContentIntent(pending)
                .setAutoCancel(true)
                .build()
            context.getSystemService(NotificationManager::class.java)
                .notify(event.agent.hashCode(), notification)
        }
    }
}
