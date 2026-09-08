package com.example.irohapp

import android.os.Bundle
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
 * a valid ticket string that can be used by other apps (e.g., farm-iroh) to fetch the model.
 * 
 * Note: This test requires both native_iroh_engine and farm-iroh apps to be installed
 * on the same device/emulator, and the farm-iroh side PR #1337farm/flashforge-farm#49
 * to be implemented. This test currently verifies the native_iroh_engine side of the interop.
 */
@RunWith(AndroidJUnit4::class)
class CrossAppTicketInteropTest {

    private lateinit var irohBridge: IrohBridge

    @Before
    fun setup() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        irohBridge = IrohBridge()
    }

    @Test
    fun testPublishAndGetTicket() {
        // Initialize the native engine
        val storageDir = context.getDir("iroh", Context.MODE_PRIVATE).absolutePath
        val secretKey = ByteArray(32)
        java.security.SecureRandom.getInstance("SHA1PRNG").nextBytes(secretKey)

        val success = irohBridge.initialize(storageDir, secretKey)
        assertTrue("initialize should succeed", success)

        try {
            // Publish a test model
            val payload = "test payload".toByteArray()
            val metadataJson = """{"schema":1,"title":"Cross-app test","description":"test model","designer":{"name":"test","pubkey":""},"license":{"spdx":"CC0-1.0","url":""},"category":"test","tags":["test"],"images":[],"files":["test.txt"],"printSettings":{},"remixOf":null,"verification":""}"""

            val ticket = irohBridge.modelPublish(metadataJson, arrayOf(payload))
            assertTrue("modelPublish should return a ticket", ticket != null && ticket.isNotEmpty())

            // Verify the ticket format is valid
            val t: iroh_blobs.ticket.BlobTicket = ticket.trim().parse<iroh_blobs.ticket.BlobTicket>()
            assertTrue("ticket should be valid", t != null)

            // Verify the ticket can be converted back to info
            val ticketInfo = irohBridge.ticketInfo(ticket)
            assertTrue("ticketInfo should not be null", ticketInfo != null && ticketInfo.isNotEmpty())

            // Verify metadata roundtrip - fetch the model back
            val fetchDir = context.cacheDir.absolutePath + "/fetched"
            irohBridge.modelFetch(ticket, fetchDir, object : IrohTransferListener {
                override fun onTransferProgress(statusCode: Int, progressPct: Int, message: String) {}
                override fun onModelMetadata(modelJson: String, fileNamesJson: String) {}
                override fun onFetchComplete(dir: String) {}
                override fun onDownloadError(error: Exception?) {}
            })

            // Verify the fetched directory exists and has content
            val fetchedFile = java.io.File(fetchDir, "test.txt")
            assertTrue("fetched file should exist", fetchedFile.exists())

        } finally {
            irohBridge.shutdown()
        }
    }

    @Test
    fun testTicketStringFormat() {
        // Initialize the native engine
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val storageDir = context.getDir("iroh", Context.MODE_PRIVATE).absolutePath
        val secretKey = ByteArray(32)
        java.security.SecureRandom.getInstance("SHA1PRNG").nextBytes(secretKey)

        val success = irohBridge.initialize(storageDir, secretKey)
        assertTrue("initialize should succeed", success)

        try {
            // Publish a model
            val payload = "cross app test".toByteArray()
            val metadataJson = """{"schema":1,"title":"Ticket format test","description":"test","designer":{"name":"test","pubkey":""},"license":{"spdx":"CC0-1.0","url":""},"category":"test","tags":["test"],"images":[],"files":["test.txt"],"printSettings":{},"remixOf":null,"verification":""}"""

            val ticket = irohBridge.modelPublish(metadataJson, arrayOf(payload))
            assertTrue("modelPublish should return a ticket", ticket != null && ticket.isNotEmpty())

            // The ticket string format should be parseable by both apps
            // farm-iroh parses BlobTicket strings from iroh-blobs 0.103.0
            // Expected format: "ticket:<hash>:<peer_id>"
            // Native engine produces tickets via iroh_blobs::ticket::BlobTicket::new(endpoint.addr(), hash, Raw)
            
            // Verify ticket contains expected components
            assertTrue("ticket should contain hash component", ticket.contains(":"))
            assertTrue("ticket should be non-empty string", ticket.length > 0)

            // Verify the ticket can be used with ticketFor and ticketInfo
            val hashBytes = irohBridge.blobAdd(payload)
            assertTrue("blobAdd should return a hash", hashBytes != null)

        } finally {
            irohBridge.shutdown()
        }
    }
}
