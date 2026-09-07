package com.example.irohapp

interface IrohProgressListener {
    fun onTransferProgress(statusCode: Int, progressPct: Int, message: String)
}

object IrohBridge {
    init {
        System.loadLibrary("native_iroh_engine")
    }

    external fun initializeAndDownload(
        storageDir: String,
        ticketStr: String,
        callback: IrohProgressListener
    )

    external fun cancelDownload(): Boolean
}
