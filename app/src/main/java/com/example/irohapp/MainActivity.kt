package com.example.irohapp

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import java.security.SecureRandom
import kotlin.concurrent.thread

class MainActivity : AppCompatActivity(), IrohTransferListener {
    private lateinit var log: TextView
    private lateinit var ticketInput: EditText
    private lateinit var keywordInput: EditText

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        val pad = (16 * resources.displayMetrics.density).toInt()
        root.setPadding(pad, pad, pad, pad)

        ticketInput = EditText(this).apply { hint = "model/announce ticket" }
        keywordInput = EditText(this).apply { hint = "search keyword" }
        log = TextView(this).apply { text = "ready\n" }

        fun btn(label: String, fn: () -> Unit) {
            root.addView(Button(this).apply {
                text = label
                setOnClickListener { thread { fn() } }
            })
        }

        root.addView(ticketInput)
        root.addView(keywordInput)
        btn("1. Initialize") { doInit() }
        btn("2. Publish demo model") { doPublish() }
        btn("3. Announce (show ticket)") { doAnnounce() }
        btn("4. Search") { doSearch() }
        btn("5. Fetch ticket") { doFetch() }
        btn("6. Merge announcement") { doMerge() }
        btn("7. Known peers") { doPeers() }
        btn("Start Sync Service") { doStartSyncService() }
        val scroll = ScrollView(this)
        scroll.addView(log)
        root.addView(scroll, LinearLayout.LayoutParams(
            LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
        setContentView(root)
    }

    private fun engineDir() = getDir("iroh", Context.MODE_PRIVATE).absolutePath

    private fun ensureInit(): Boolean {
        if (!IrohBridge.initialize(engineDir(), EngineSecret.loadOrCreate(this))) {
            append("initialize failed\n")
            return false
        }
        return true
    }

    private fun doInit() {
        if (!ensureInit()) return
        val id = IrohBridge.endpointId().joinToString("") { "%02x".format(it) }
        append("endpoint: $id\n")
    }

    private fun doPublish() {
        if (!ensureInit()) return
        val payload = "hello farmglow ${(0..999).joinToString(",")}\n".toByteArray()
        val meta = """{"schema":1,"title":"Demo Cube","description":"roundtrip demo model","designer":{"name":"demo","pubkey":""},"license":{"spdx":"CC0-1.0","url":""},"category":"demo","tags":["demo","test"],"images":[],"files":["demo.txt"],"printSettings":{"layerHeight":0.2,"infill":0.2,"supports":false,"notes":""},"remixOf":null,"verification":""}"""
        val ticket = IrohBridge.modelPublish(meta, arrayOf(payload))
        append("published ticket: $ticket\n")
        runOnUiThread { ticketInput.setText(ticket ?: "") }
    }

    private fun doAnnounce() {
        if (!ensureInit()) return
        val ticket = IrohBridge.syncAnnounce()
        append("announce ticket: $ticket\n")
        runOnUiThread { ticketInput.setText(ticket ?: "") }
    }

    private fun doSearch() {
        if (!ensureInit()) return
        val kw = keywordInput.text.toString().ifEmpty { "demo" }
        val res = IrohBridge.searchQuery(kw)
        append("search '$kw': ${res.size} hit(s)\n")
        res.take(5).forEach { append("  $it\n") }
    }

    private fun doFetch() {
        if (!ensureInit()) return
        val ticket = ticketInput.text.toString()
        if (ticket.isBlank()) {
            append("enter a ticket first\n")
            return
        }
        IrohBridge.modelFetch(ticket, "$filesDir/p2p", this)
        append("fetch started...\n")
    }

    private fun doMerge() {
        if (!ensureInit()) return
        val ticket = ticketInput.text.toString()
        if (ticket.isBlank()) {
            append("enter an announcement ticket first\n")
            return
        }
        append("merge: ${IrohBridge.syncMerge(ticket)}\n")
    }

    private fun doPeers() {
        if (!ensureInit()) return
        append("peers: ${IrohBridge.knownPeers().joinToString()}\n")
    }

    private fun doStartSyncService() {
        val ticket = ticketInput.text.toString()
        if (ticket.isBlank()) {
            append("enter a ticket first\n")
            return
        }
        val intent = Intent(this, IrohDaemonService::class.java).apply {
            putExtra(IrohDaemonService.EXTRA_TICKET, ticket)
        }
        ContextCompat.startForegroundService(this, intent)
        append("sync service started for ticket...\n")
    }

    private fun append(s: String) {
        runOnUiThread { log.append(s) }
    }

    override fun onTransferProgress(
        statusCode: Int,
        progressPct: Int,
        downloadedBytes: Long,
        totalBytes: Long,
        message: String
    ) {
        append("[$statusCode $progressPct%] $message\n")
    }

    override fun onModelMetadata(modelJson: String, fileNamesJson: String) {
        append("metadata: $fileNamesJson\n")
    }

    override fun onFetchComplete(dir: String) {
        append("done: $dir\n")
    }
}
