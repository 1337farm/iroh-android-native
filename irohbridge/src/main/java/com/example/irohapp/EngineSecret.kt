package com.example.irohapp

import android.content.Context
import java.security.SecureRandom

object EngineSecret {
    private const val PREFS_NAME = "iroh_engine"
    private const val KEY_SECRET = "secret"

    fun loadOrCreate(context: Context): ByteArray {
        val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        val hex = prefs.getString(KEY_SECRET, null)
        if (hex != null && hex.length == 64) {
            return hex.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
        }
        val secret = ByteArray(32)
        SecureRandom().nextBytes(secret)
        prefs.edit().putString(KEY_SECRET, secret.joinToString("") { "%02x".format(it) }).apply()
        return secret
    }
}
