package com.example.irohapp

interface IrohTransferListener {
    fun onTransferProgress(
        statusCode: Int,
        progressPct: Int,
        downloadedBytes: Long,
        totalBytes: Long,
        message: String
    )

    fun onModelMetadata(modelJson: String, fileNamesJson: String)
    fun onFetchComplete(dir: String)
}

/** Callbacks for the USB reverse-pairing accept loop (ALPN farm/pair/0). */
interface IPairListener {
    fun onPairResult(peerNodeIdHex: String, tokenHex: String, ok: Boolean)
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

    // Download a single blob by ticket and return its raw bytes (null on error);
    // used for fetching standalone blob payloads (e.g. profile bundles).
    external fun blobFetch(ticket: String): ByteArray?

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

    // USB reverse-pairing (farm/pair): accept pair-beacons from printers that
    // dial this phone first. tokensJson is a JSON array of pending token hex
    // strings; onPairResult fires per beacon (tokenHex empty when rejected).
    external fun pairAccept(alpn: String, tokensJson: String, callback: IPairListener): Boolean
    external fun pairStop()

    // Dial a peer NodeId with an ALPN, send payloadJson, return the one-shot
    // response string. Throws RuntimeException on transport failure.
    // relayUrl empty = default relays. timeoutSecs clamped to 5..300.
    external fun pairDial(
        nodeIdHex: String,
        relayUrl: String,
        alpn: String,
        payloadJson: String,
        timeoutSecs: Int
    ): String?

    @Deprecated("Use initialize() + modelFetch(); kept for compatibility. Now actually writes the file.")
    external fun initializeAndDownload(
        storageDir: String,
        ticketStr: String,
        callback: IrohTransferListener
    )

    external fun cancelDownload(): Boolean
}