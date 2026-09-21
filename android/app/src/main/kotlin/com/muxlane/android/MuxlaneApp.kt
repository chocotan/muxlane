package com.muxlane.android

import android.app.Application

class MuxlaneApp : Application() {
    lateinit var session: MuxlaneSession
        private set

    override fun onCreate() {
        super.onCreate()
        session = MuxlaneSession(this)
    }
}
