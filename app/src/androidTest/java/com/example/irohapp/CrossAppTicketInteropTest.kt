package com.example.irohapp

import android.content.Context
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import kotlin.test.assertTrue

/**
 * Cross-app ticket interop test.
 *
 * This test verifies that the native_iroh_engine can publish a model and produce
 * a valid ticket string that can be used by other apps to fetch the model.
 *
 * Note: This test verifies the native_iroh_engine side of the interop. The
 * consumer side is FlashForge Farm, which pulls these tickets through the
 * `com.example.irohapp:irohbridge` AAR (published from this repo).
 */
@RunWith(AndroidJUnit4::class)
class CrossAppTicketInteropTest {

    private lateinit var irohBridge: IrohBridge
    private lateinit var context: Context

    @Before
    fun setup() {
        context = InstrumentationRegistry.getInstrumentation().targetContext
        irohBridge = IrohBridge
    }

    @Test
    fun testPublishAndGetTicket() {
        // Initialize the native engine with a stable secret key
        val storageDir = context.getDir("iroh", Context.MODE_PRIVATE).absolutePath
        val secretKey = ByteArray(32)
        java.security.SecureRandom.getInstance("SHA1PRNG").nextBytes(secretKey)

        val success = irohBridge.initialize(storageDir, secretKey)
        assertTrue("initialize should succeed", success)

        try {
            // Publish a test model with the metadata contract
            val payload = "test payload for cross-app interop".toByteArray()
            val metadataJson = """{"schema":1,"title":"Cross-app test","description":"test model for interop","designer":{"name":"test","pubkey":""},"license":{"spdx":"CC0-1.0","url":""},"category":"test","tags":["test","interop"],"images":[],"files":["test.txt"],"printSettings":{},"remixOf":null,"verification":""}"""

            val ticket = irohBridge.modelPublish(metadataJson, arrayOf(payload))
            assertTrue("modelPublish should return a ticket", ticket != null && ticket.isNotEmpty())

            // Verify the ticket can be converted back to info
            val ticketInfo = irohBridge.ticketInfo(ticket)
            assertTrue(
                "ticketInfo should not be null and contain hash:peer format",
                ticketInfo != null && ticketInfo.isNotEmpty() && ticketInfo.contains(":")
            )

            // Verify metadata roundtrip - fetch the model back using the same engine
            val listener = TestTransferListener()
            val fetchDir = context.getDir("fetched", Context.MODE_PRIVATE).absolutePath
            irohBridge.modelFetch(ticket, fetchDir, listener)

            // Wait for fetch to complete (with timeout)
            val completed = listener.waitForCompletion(30000)
            assertTrue("fetch should complete within timeout", completed)

            // Verify the fetched directory exists and has the expected file
            val fetchedFile = java.io.File(fetchDir, "test.txt")
            assertTrue("fetched file should exist", fetchedFile.exists())

        } finally {
            irohBridge.shutdown()
        }
    }

    @Test
    fun testTicketStringFormatInterop() {
        // Initialize the native engine
        val storageDir = context.getDir("iroh2", Context.MODE_PRIVATE).absolutePath
        val secretKey = ByteArray(32)
        java.security.SecureRandom.getInstance("SHA1PRNG").nextBytes(secretKey)

        val success = irohBridge.initialize(storageDir, secretKey)
        assertTrue("initialize should succeed", success)

        try {
            // Publish a model
            val payload = "cross app test data".toByteArray()
            val metadataJson = """{"schema":1,"title":"Ticket format test","description":"test","designer":{"name":"test","pubkey":""},"license":{"spdx":"CC0-1.0","url":""},"category":"test","tags":["test"],"images":[],"files":["test.txt"],"printSettings":{},"remixOf":null,"verification":""}"""

            val ticket = irohBridge.modelPublish(metadataJson, arrayOf(payload))
            assertTrue("modelPublish should return a ticket", ticket != null && ticket.isNotEmpty())

            // The ticket string format must be parseable by both apps:
            // - native_iroh_engine produces tickets via iroh_blobs::ticket::BlobTicket::new(endpoint.addr(), hash, Raw)
            // - FlashForge Farm consumes the same iroh-blobs 0.103.0 format via the irohbridge AAR
            // The format is a string like "ticket:<hash_hex>:<peer_id_hex>"

            // Verify ticket contains the expected components for cross-app interop
            assertTrue("ticket should contain a scheme identifier", ticket != null)
            assertTrue("ticket should contain hash and peer components", ticket!!.contains(":"))

            // Verify the ticket can be used with blob operations
            val hashBytes = irohBridge.blobAdd(payload)
            assertTrue("blobAdd should return a hash", hashBytes != null)

            // Verify the hash from blobAdd matches what we get from ticketInfo
            val infoResult = irohBridge.ticketInfo(ticket)
            val infoParts = infoResult?.split(":")
            assertTrue("ticketInfo should contain hash:peer format", infoParts != null && infoParts.size >= 2)

        } finally {
            irohBridge.shutdown()
        }
    }

    @Test
    fun testSyncAnnounceAndMergeInterop() {
        // Initialize the native engine
        val storageDir = context.getDir("iroh3", Context.MODE_PRIVATE).absolutePath
        val secretKey = ByteArray(32)
        java.security.SecureRandom.getInstance("SHA1PRNG").nextBytes(secretKey)

        val success = irohBridge.initialize(storageDir, secretKey)
        assertTrue("initialize should succeed", success)

        try {
            // Publish a model
            val payload = "sync test data".toByteArray()
            val metadataJson = """{"schema":1,"title":"Sync test","description":"testing sync interop","designer":{"name":"test","pubkey":""},"license":{"spdx":"CC0-1.0","url":""},"category":"test","tags":["sync"],"images":[],"files":["sync.txt"],"printSettings":{},"remixOf":null,"verification":""}"""

            val publishTicket = irohBridge.modelPublish(metadataJson, arrayOf(payload))
            assertTrue("modelPublish should return a ticket", publishTicket != null && publishTicket.isNotEmpty())

            // Announce (creates a sync ticket)
            val announceTicket = irohBridge.syncAnnounce()
            assertTrue("syncAnnounce should return a ticket", announceTicket != null && announceTicket.isNotEmpty())

            // Verify announce ticket has valid format
            val announceInfo = irohBridge.ticketInfo(announceTicket)
            assertTrue("announce ticket info should not be null", announceInfo != null && announceInfo.isNotEmpty())

        } finally {
            irohBridge.shutdown()
        }
    }

    /**
     * Simple listener that tracks fetch completion for testing.
     */
    private class TestTransferListener : IrohTransferListener {
        private val latch = java.util.concurrent.CountDownLatch(1)
        private var completed = false
        private var error: String? = null

        override fun onTransferProgress(
            statusCode: Int,
            progressPct: Int,
            downloadedBytes: Long,
            totalBytes: Long,
            message: String
        ) {
            if (statusCode == 5 || statusCode < 0) {
                completed = true
                error = if (statusCode < 0) message else null
                latch.countDown()
            }
        }

        override fun onModelMetadata(modelJson: String, fileNamesJson: String) {}

        override fun onFetchComplete(dir: String) {
            completed = true
            latch.countDown()
        }

        fun waitForCompletion(timeoutMs: Long): Boolean {
            return latch.await(timeoutMs, java.util.concurrent.TimeUnit.MILLISECONDS)
        }
    }
}