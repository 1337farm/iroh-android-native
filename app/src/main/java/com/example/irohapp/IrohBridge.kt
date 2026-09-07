package com.example.irohapp

interface IrohTransferListener {
    fun onTransferProgress(statusCode: Int, progressPct: Int, message: String)
    fun onModelMetadata(modelJson: String, fileNamesJson: String)
    fun onFetchComplete(dir: String)
}

object IrohBridge {
    init {
        System.loadLibrary("native_iroh_engine")
    }

    external fun initialize(storageDir: String, secretKey: ByteArray?): Boolean
    external fun shutdown()
    external fun endpointId(): ByteArray
    external fun secretKey(): ByteArray

    external fun blobAdd(data: ByteArray): ByteArray?
    external fun blobGet(hash: ByteArray): ByteArray?
    external fun blobHas(hash: ByteArray): Boolean
    external fun ticketFor(hash: ByteArray): String?
    external fun ticketInfo(ticket: String): String?

    external fun modelPublish(metadataJson: String, files: Array<ByteArray>): String?
    external fun modelPublishFiles(metadataJson: String, paths: Array<String>, callback: IrohTransferListener): String?
    external fun modelFetch(ticket: String, dir: String, callback: IrohTransferListener)
    external fun downloadToPath(storageDir: String, ticket: String, fileName: String, callback: IrohTransferListener)
    external fun cancelFetch(): Boolean

    external fun searchQuery(keyword: String): Array<String>
    external fun searchGetMetadata(modelHashHex: String): String?
    external fun syncAnnounce(): String?
    external fun syncMerge(ticket: String): String?
    external fun knownPeers(): Array<String>

    @Deprecated("Use initialize() + modelFetch(); kept for compatibility. Now actually writes the file.")
    external fun initializeAndDownload(
        storageDir: String,
        ticketStr: String,
        callback: IrohTransferListener
    )

    external fun cancelDownload(): Boolean
}
