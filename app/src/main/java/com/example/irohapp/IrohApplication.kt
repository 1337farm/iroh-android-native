package com.example.irohapp

import android.app.Application
import android.os.Environment
import java.io.File
import java.io.FileWriter
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class IrohApplication : Application() {
    override fun onCreate() {
        super.onCreate()

        val defaultHandler = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, throwable ->
            logErrorToFile(throwable)
            defaultHandler?.uncaughtException(thread, throwable)
        }
    }

    private fun logErrorToFile(throwable: Throwable) {
        try {
            val downloadsDir = Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS)
            if (!downloadsDir.exists()) {
                downloadsDir.mkdirs()
            }
            val logFile = File(downloadsDir, "irohapp_errors.log")

            val timestamp = SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US).format(Date())
            val writer = FileWriter(logFile, true)

            writer.append("======================\n")
            writer.append("Time: $timestamp\n")
            writer.append("Error: ${throwable.message}\n")
            writer.append("Stacktrace:\n")
            writer.append(Log.getStackTraceString(throwable))
            writer.append("\n======================\n\n")
            writer.flush()
            writer.close()
        } catch (e: Exception) {
            Log.e("IrohApplication", "Failed to write error to log file", e)
        }
    }
}
