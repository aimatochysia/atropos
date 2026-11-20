package io.github.aimatochysia.atropos

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.provider.Settings
import android.view.WindowManager
import java.io.File
import androidx.core.content.FileProvider
import io.flutter.embedding.android.FlutterFragmentActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import io.flutter.plugin.common.MethodChannel.Result

/**
 * Blocks screenshots and the recents thumbnail, offers biometric unlock
 * (see [BiometricVault]), exposes the "All files access"
 * permission to Dart (vaults can live in any folder) and hands verified update
 * APKs to the system installer.
 */
class MainActivity : FlutterFragmentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        // Decrypted content must never end up in screenshots, screen
        // recordings or the recent-apps preview.
        window.setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE)
        super.onCreate(savedInstanceState)
        // The Rust core plays audio itself and needs the app context for it.
        try {
            System.loadLibrary("atropos_core")
            initNative(applicationContext)
        } catch (e: UnsatisfiedLinkError) {
            // Audio will report that it cannot start; everything else works.
        }
    }

    private external fun initNative(context: android.content.Context)

    private var pendingPick: Result? = null
    private var nfcCards: NfcCards? = null

    override fun onPause() {
        // Reader mode only works in the foreground; a card in progress is lost.
        nfcCards?.cancel()
        super.onPause()
    }

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "atropos/storage")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "hasAllFilesAccess" -> result.success(hasAccess())
                    "requestAllFilesAccess" -> {
                        requestAccess()
                        result.success(null)
                    }
                    // Opens the system folder picker and answers with a real
                    // filesystem path (the Rust core works on real paths, not content:// URIs).
                    "pickDirectory" -> {
                        pendingPick?.success(null)
                        pendingPick = result
                        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
                            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                        }
                        try {
                            startActivityForResult(intent, PICK_DIRECTORY)
                        } catch (e: Exception) {
                            pendingPick = null
                            result.error("picker", e.message, null)
                        }
                    }
                    // Opens the system file picker and answers with the picked
                    // file's real path, or null when it has none (then Dart falls
                    // back to a temporary copy).
                    "pickFilePath" -> {
                        pendingPick?.success(null)
                        pendingPick = result
                        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
                            addCategory(Intent.CATEGORY_OPENABLE)
                            type = "*/*"
                            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                        }
                        try {
                            startActivityForResult(intent, PICK_FILE)
                        } catch (e: Exception) {
                            pendingPick = null
                            result.error("picker", e.message, null)
                        }
                    }
                    else -> result.notImplemented()
                }
            }
        val nfc = NfcCards(this)
        nfcCards = nfc
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "atropos/nfc")
            .setMethodCallHandler { call, result ->
                if (call.method == "cancel") {
                    nfc.cancel()
                    result.success(null)
                } else {
                    nfc.handle(call, result)
                }
            }
        val biometric = BiometricVault(this)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "atropos/biometric")
            .setMethodCallHandler { call, result -> biometric.handle(call, result) }
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "atropos/install")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    // Returns "started", or "permission" when the person first has to
                    // allow installs from Atropos (the settings page is opened).
                    "installApk" -> {
                        val path = call.argument<String>("path")
                        if (path == null) {
                            result.error("args", "path missing", null)
                        } else {
                            try {
                                result.success(installApk(File(path)))
                            } catch (e: Exception) {
                                result.error("install", e.message, null)
                            }
                        }
                    }
                    else -> result.notImplemented()
                }
            }
    }

    @Deprecated("Uses the platform activity result API for a single request.")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (requestCode != PICK_DIRECTORY && requestCode != PICK_FILE) {
            super.onActivityResult(requestCode, resultCode, data)
            return
        }
        val result = pendingPick ?: return
        pendingPick = null
        val uri = data?.data
        if (resultCode != RESULT_OK || uri == null) {
            result.success(null)
            return
        }
        if (requestCode == PICK_FILE) {
            val path = documentUriToPath(uri)
            if (path != null) {
                result.success(path)
                return
            }
            // No real path (cloud or app storage): hand over a temporary copy,
            // marked "copy:" so Dart deletes it afterwards.
            Thread {
                try {
                    val copy = copyToCache(uri)
                    runOnUiThread { result.success("copy:" + copy.path) }
                } catch (e: Exception) {
                    runOnUiThread { result.error("picker", e.message, null) }
                }
            }.start()
            return
        }
        val path = treeUriToPath(uri)
        if (path == null) {
            result.error(
                "unsupported",
                "That location is not a folder on this device's storage. Pick a folder on internal storage or an SD card.",
                null,
            )
        } else {
            result.success(path)
        }
    }

    /** Maps a document-tree URI from the system picker to a filesystem path. */
    private fun treeUriToPath(uri: Uri): String? {
        val docId = try {
            DocumentsContract.getTreeDocumentId(uri)
        } catch (_: Exception) {
            return null
        }
        return docIdToPath(uri.authority, docId)?.takeIf { File(it).isDirectory }
    }

    /** Maps a picked document's URI to its filesystem path, when it has one. */
    private fun documentUriToPath(uri: Uri): String? {
        val docId = try {
            DocumentsContract.getDocumentId(uri)
        } catch (_: Exception) {
            return null
        }
        return docIdToPath(uri.authority, docId)?.takeIf { File(it).isFile }
    }

    private fun copyToCache(uri: Uri): File {
        var name = "picked.atropos"
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { c -> if (c.moveToFirst()) c.getString(0)?.let { name = it } }
        val dir = File(cacheDir, "picked").apply { mkdirs() }
        val safe = name.replace('/', '_').ifEmpty { "picked.atropos" }
        val out = File(dir, "${System.currentTimeMillis()}-$safe")
        contentResolver.openInputStream(uri).use { input ->
            requireNotNull(input) { "The file could not be read." }
            out.outputStream().use { input.copyTo(it) }
        }
        return out
    }

    private fun docIdToPath(authority: String?, docId: String): String? {
        return when (authority) {
            "com.android.externalstorage.documents" -> {
                val volume = docId.substringBefore(':')
                val rel = docId.substringAfter(':', "")
                val base = if (volume.equals("primary", ignoreCase = true)) {
                    Environment.getExternalStorageDirectory().path
                } else {
                    "/storage/$volume"
                }
                if (rel.isEmpty()) base else "$base/$rel"
            }
            "com.android.providers.downloads.documents" -> when {
                docId.startsWith("raw:") -> docId.removePrefix("raw:")
                docId == "downloads" -> Environment
                    .getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS).path
                else -> null
            }
            else -> null
        }
    }

    private fun installApk(file: File): String {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O &&
            !packageManager.canRequestPackageInstalls()
        ) {
            startActivity(
                Intent(
                    Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:$packageName"),
                ),
            )
            return "permission"
        }
        val uri = FileProvider.getUriForFile(this, "$packageName.updates", file)
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, "application/vnd.android.package-archive")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        startActivity(intent)
        return "started"
    }

    private fun hasAccess(): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            Environment.isExternalStorageManager()
        } else {
            checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) ==
                PackageManager.PERMISSION_GRANTED
        }

    private fun requestAccess() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val intent = Intent(
                Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                Uri.parse("package:$packageName"),
            )
            try {
                startActivity(intent)
            } catch (_: Exception) {
                startActivity(Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
            }
        } else {
            requestPermissions(
                arrayOf(
                    Manifest.permission.READ_EXTERNAL_STORAGE,
                    Manifest.permission.WRITE_EXTERNAL_STORAGE,
                ),
                1,
            )
        }
    }
}

private const val PICK_DIRECTORY = 4101
private const val PICK_FILE = 4102
