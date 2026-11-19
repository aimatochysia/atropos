package io.github.aimatochysia.atropos

import android.app.Activity
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.nfc.tech.IsoDep
import android.os.Handler
import android.os.Looper
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel.Result
import java.io.IOException
import java.util.concurrent.Executors

/**
 * Smartcards over NFC (ISO/IEC 14443-4, IsoDep). This class only moves bytes:
 * the Rust core builds every APDU and checks every answer.
 */
class NfcCards(private val activity: Activity) {
    private val main = Handler(Looper.getMainLooper())
    private val io = Executors.newSingleThreadExecutor()
    private var waiting: Result? = null
    private var card: IsoDep? = null

    fun handle(call: MethodCall, result: Result) {
        when (call.method) {
            "status" -> result.success(status())
            "waitForCard" -> waitForCard(result)
            "transceive" -> {
                val apdu = call.argument<ByteArray>("apdu")
                val c = card
                if (apdu == null || c == null) {
                    result.success(ByteArray(0))
                    return
                }
                io.execute {
                    val r = try {
                        c.transceive(apdu)
                    } catch (_: IOException) {
                        ByteArray(0)
                    } catch (_: IllegalStateException) {
                        ByteArray(0)
                    }
                    main.post { result.success(r) }
                }
            }
            "close" -> {
                close()
                result.success(null)
            }
            else -> result.notImplemented()
        }
    }

    /** "ready", "off" (NFC turned off) or "none" (no NFC hardware). */
    private fun status(): String {
        val a = NfcAdapter.getDefaultAdapter(activity) ?: return "none"
        return if (a.isEnabled) "ready" else "off"
    }

    /** Answers true once a smartcard is held to the phone, false if cancelled. */
    private fun waitForCard(result: Result) {
        val a = NfcAdapter.getDefaultAdapter(activity)
        if (a == null || !a.isEnabled) {
            result.error("nfc", "NFC is off. Turn it on in the system settings.", null)
            return
        }
        waiting?.success(false)
        waiting = result
        close()
        val flags = NfcAdapter.FLAG_READER_NFC_A or NfcAdapter.FLAG_READER_NFC_B or
            NfcAdapter.FLAG_READER_SKIP_NDEF_CHECK or NfcAdapter.FLAG_READER_NO_PLATFORM_SOUNDS
        a.enableReaderMode(activity, { tag: Tag -> onTag(tag) }, flags, null)
    }

    private fun onTag(tag: Tag) {
        val dep = IsoDep.get(tag)
        val r = waiting ?: return
        if (dep == null) {
            main.post {
                if (waiting === r) {
                    waiting = null
                    stopReader()
                    r.error("nfc", "That is not a smartcard. Hold a contactless security card to the phone.", null)
                }
            }
            return
        }
        try {
            dep.connect()
            // ECDH on a card can take a second or two.
            dep.timeout = 5000
        } catch (e: IOException) {
            main.post {
                if (waiting === r) {
                    waiting = null
                    stopReader()
                    r.error("nfc", "Lost the card. Hold it still against the phone and try again.", null)
                }
            }
            return
        }
        card = dep
        main.post {
            if (waiting === r) {
                waiting = null
                r.success(true)
            }
        }
    }

    private fun stopReader() {
        try {
            NfcAdapter.getDefaultAdapter(activity)?.disableReaderMode(activity)
        } catch (_: Exception) {
        }
    }

    fun close() {
        try {
            card?.close()
        } catch (_: IOException) {
        }
        card = null
        stopReader()
    }

    /** Stops waiting (the person cancelled). */
    fun cancel() {
        waiting?.success(false)
        waiting = null
        close()
    }
}
