package com.muxlane.android

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import muxlane.protocol.AgentInstance
import muxlane.protocol.AgentStatus
import muxlane.protocol.AgentStatusEvent
import muxlane.protocol.isAlert

class RelayService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var stateJob: Job? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        ensureChannels(this)
        startForeground(ONGOING_ID, ongoingNotification(getString(R.string.service_restoring)))
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val session = (application as MuxlaneApp).session
        if (session.state.value.pairing == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        session.ensureConnected()
        if (stateJob?.isActive != true) {
            stateJob = scope.launch {
                session.state.collect { state ->
                    val text = when {
                        state.connected -> getString(R.string.service_connected, state.pairing?.machineName ?: getString(R.string.computer))
                        state.connecting -> getString(R.string.service_connecting, state.pairing?.machineName ?: getString(R.string.computer))
                        else -> getString(R.string.service_retrying)
                    }
                    startForeground(ONGOING_ID, ongoingNotification(text))
                }
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        (application as MuxlaneApp).session.stop()
        scope.cancel()
        super.onDestroy()
    }

    private fun ongoingNotification(text: String): Notification {
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CONNECTION_CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .build()
    }

    companion object {
        const val CONNECTION_CHANNEL = "muxlane-connection"
        const val ALERT_CHANNEL = "muxlane-status"
        const val EXTRA_AGENT = "agent"
        const val EXTRA_HOST = "host"
        private const val ONGOING_ID = 1

        fun ensureChannels(context: Context) {
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannels(
                listOf(
                    NotificationChannel(CONNECTION_CHANNEL, context.getString(R.string.channel_connection), NotificationManager.IMPORTANCE_LOW),
                    NotificationChannel(ALERT_CHANNEL, context.getString(R.string.channel_alerts), NotificationManager.IMPORTANCE_DEFAULT),
                ),
            )
        }

        fun notifyStatus(context: Context, hostId: String, agent: AgentInstance?, event: AgentStatusEvent) {
            if (!event.to.isAlert()) return
            if (Build.VERSION.SDK_INT >= 33 &&
                ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
            ) return
            ensureChannels(context)
            val title = when (event.to) {
                AgentStatus.BLOCKED -> context.getString(R.string.notification_waiting)
                AgentStatus.DONE -> context.getString(R.string.notification_done)
                AgentStatus.FAILED -> context.getString(R.string.notification_failed)
                else -> event.to.name
            }
            val body = event.message ?: agent?.title ?: event.agent
            val open = Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
                .putExtra(EXTRA_HOST, hostId)
                .putExtra(EXTRA_AGENT, event.agent)
            val pending = PendingIntent.getActivity(
                context,
                event.agent.hashCode(),
                open,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val notification = NotificationCompat.Builder(context, ALERT_CHANNEL)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(title)
                .setContentText(body)
                .setStyle(NotificationCompat.BigTextStyle().bigText(body))
                .setContentIntent(pending)
                .setAutoCancel(true)
                .setCategory(NotificationCompat.CATEGORY_STATUS)
                .build()
            context.getSystemService(NotificationManager::class.java)
                .notify(event.agent.hashCode(), notification)
        }
    }
}
