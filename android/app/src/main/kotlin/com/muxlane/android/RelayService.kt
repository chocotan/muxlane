package com.muxlane.android

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.RemoteInput
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import muxlane.chat.ApprovalReply
import muxlane.chat.PendingApproval
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
        const val APPROVAL_CHANNEL = "muxlane-approvals"
        const val EXTRA_AGENT = "agent"
        const val EXTRA_HOST = "host"
        const val EXTRA_APPROVAL_ID = "approval_id"
        const val EXTRA_ACTION_KIND = "action_kind"
        const val EXTRA_OPTION_ID = "option_id"
        const val KEY_TEXT_REPLY = "muxlane_approval_reply"
        const val ACTION_ALLOW = "allow"
        const val ACTION_DENY = "deny"
        const val ACTION_TEXT = "text"
        private const val ONGOING_ID = 1

        fun ensureChannels(context: Context) {
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannels(
                listOf(
                    NotificationChannel(CONNECTION_CHANNEL, context.getString(R.string.channel_connection), NotificationManager.IMPORTANCE_LOW),
                    NotificationChannel(ALERT_CHANNEL, context.getString(R.string.channel_alerts), NotificationManager.IMPORTANCE_DEFAULT),
                    NotificationChannel(APPROVAL_CHANNEL, context.getString(R.string.channel_approvals), NotificationManager.IMPORTANCE_HIGH),
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

        fun notifyApproval(context: Context, hostId: String, agent: AgentInstance?, approval: PendingApproval) {
            if (Build.VERSION.SDK_INT >= 33 &&
                ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
            ) return
            ensureChannels(context)
            val title = "${agent?.title ?: approval.agent} · ${approval.title}"
            val body = listOfNotNull(
                approval.toolName?.let { "工具：$it" },
                approval.commandOrInput?.take(160),
                approval.message,
            ).joinToString("\n").ifBlank { context.getString(R.string.approval_notification_title) }

            val openIntent = Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
                .putExtra(EXTRA_HOST, hostId)
                .putExtra(EXTRA_AGENT, approval.agent)
            val openPending = PendingIntent.getActivity(
                context,
                approval.id.hashCode(),
                openIntent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )

            val allowOption = approval.options.firstOrNull { opt ->
                val kind = opt.kind.orEmpty().lowercase()
                val label = opt.label.lowercase()
                "allow" in kind || "allow" in label || "once" in kind
            } ?: approval.options.firstOrNull()
            val denyOption = approval.options.firstOrNull { opt ->
                val kind = opt.kind.orEmpty().lowercase()
                val label = opt.label.lowercase()
                "reject" in kind || "deny" in kind || "deny" in label || "cancel" in kind
            }

            fun actionPending(reqOffset: Int, kind: String, optionId: String?): PendingIntent {
                val actionIntent = Intent(context, ApprovalActionReceiver::class.java).apply {
                    putExtra(EXTRA_HOST, hostId)
                    putExtra(EXTRA_AGENT, approval.agent)
                    putExtra(EXTRA_APPROVAL_ID, approval.id)
                    putExtra(EXTRA_ACTION_KIND, kind)
                    if (optionId != null) putExtra(EXTRA_OPTION_ID, optionId)
                }
                val flags = PendingIntent.FLAG_UPDATE_CURRENT or
                    if (kind == ACTION_TEXT && Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE
                    else PendingIntent.FLAG_IMMUTABLE
                return PendingIntent.getBroadcast(
                    context,
                    (approval.id.hashCode() * 31) + reqOffset,
                    actionIntent,
                    flags,
                )
            }

            val builder = NotificationCompat.Builder(context, APPROVAL_CHANNEL)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(title)
                .setContentText(body)
                .setStyle(NotificationCompat.BigTextStyle().bigText(body))
                .setContentIntent(openPending)
                .setAutoCancel(true)
                .setPriority(NotificationCompat.PRIORITY_HIGH)
                .setCategory(NotificationCompat.CATEGORY_MESSAGE)

            if (approval.options.isEmpty() || approval.prefill != null) {
                val remoteInput = RemoteInput.Builder(KEY_TEXT_REPLY)
                    .setLabel(context.getString(R.string.approve_reply_hint))
                    .build()
                builder.addAction(
                    NotificationCompat.Action.Builder(0, context.getString(R.string.approve_reply), actionPending(3, ACTION_TEXT, null))
                        .addRemoteInput(remoteInput)
                        .build(),
                )
            }
            if (approval.options.isEmpty() || allowOption != null) {
                builder.addAction(
                    0,
                    allowOption?.label ?: context.getString(R.string.approve_allow),
                    actionPending(1, ACTION_ALLOW, allowOption?.optionId),
                )
            }
            if (approval.options.isEmpty() || denyOption != null) {
                builder.addAction(
                    0,
                    denyOption?.label ?: context.getString(R.string.approve_deny),
                    actionPending(2, ACTION_DENY, denyOption?.optionId),
                )
            }

            context.getSystemService(NotificationManager::class.java)
                .notify(approvalNotificationId(approval.id), builder.build())
        }

        fun cancelApprovalNotification(context: Context, approvalId: String) {
            context.getSystemService(NotificationManager::class.java)
                .cancel(approvalNotificationId(approvalId))
        }

        private fun approvalNotificationId(approvalId: String): Int =
            0x40000000 xor (approvalId.hashCode() and 0x3FFFFFFF)
    }
}

class ApprovalActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val agent = intent.getStringExtra(RelayService.EXTRA_AGENT) ?: return
        val approvalId = intent.getStringExtra(RelayService.EXTRA_APPROVAL_ID) ?: return
        val kind = intent.getStringExtra(RelayService.EXTRA_ACTION_KIND) ?: return
        val optionId = intent.getStringExtra(RelayService.EXTRA_OPTION_ID)
        val reply: ApprovalReply = when (kind) {
            RelayService.ACTION_TEXT -> {
                val text = RemoteInput.getResultsFromIntent(intent)
                    ?.getCharSequence(RelayService.KEY_TEXT_REPLY)
                    ?.toString()
                    ?.trim()
                    ?: return
                ApprovalReply.Text(text)
            }
            RelayService.ACTION_ALLOW -> optionId?.let { ApprovalReply.SelectedOption(it) } ?: ApprovalReply.Confirm(true)
            RelayService.ACTION_DENY -> optionId?.let { ApprovalReply.SelectedOption(it) } ?: ApprovalReply.Confirm(false)
            else -> return
        }
        val app = context.applicationContext as? MuxlaneApp ?: return
        app.session.respondApproval(agent, approvalId, reply)
        RelayService.cancelApprovalNotification(context, approvalId)
    }
}
