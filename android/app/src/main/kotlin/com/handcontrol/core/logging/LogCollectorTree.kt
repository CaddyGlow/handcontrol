package com.handcontrol.core.logging

import android.content.Context
import android.util.Log
import timber.log.Timber
import java.io.File
import java.text.SimpleDateFormat
import java.util.*

class LogCollectorTree : Timber.Tree() {
    private val logs = Collections.synchronizedList(mutableListOf<String>())
    private val maxLogs = 1000

    override fun log(priority: Int, tag: String?, message: String, t: Throwable?) {
        val timestamp = SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US)
            .format(Date())
        val priorityStr = when (priority) {
            Log.VERBOSE -> "V"
            Log.DEBUG -> "D"
            Log.INFO -> "I"
            Log.WARN -> "W"
            Log.ERROR -> "E"
            else -> "?"
        }

        val logEntry = buildString {
            append("$timestamp $priorityStr/$tag: $message")
            if (t != null) {
                append("\n${t.stackTraceToString()}")
            }
        }

        synchronized(logs) {
            if (logs.size >= maxLogs) {
                logs.removeAt(0) // Remove oldest log
            }
            logs.add(logEntry)
        }
    }

    fun exportLogs(context: Context): File {
        val file = File(
            context.cacheDir,
            "handcontrol_logs_${System.currentTimeMillis()}.txt"
        )

        synchronized(logs) {
            file.writeText(
                logs.joinToString("\n") { redactSensitiveData(it) }
            )
        }

        return file
    }

    private fun redactSensitiveData(log: String): String {
        return log
            // Verification codes
            .replace(Regex("""verification_code=\d{3}-\d{3}"""), "verification_code=XXX-XXX")
            .replace(Regex("""verification_code"?\s*:\s*"\d{3}-\d{3}""""), "verification_code\":\"XXX-XXX\"")
            // Tokens
            .replace(Regex("""token=[\w\-_.]+"""), "token=REDACTED")
            .replace(Regex("""token"?\s*:\s*"[\w\-_.]+""""), "token\":\"REDACTED\"")
            .replace(Regex("""enrollment_token=[a-f0-9\-]+"""), "enrollment_token=REDACTED")
            .replace(Regex("""relay_token=[\w\-_.]+"""), "relay_token=REDACTED")
            // Authorization headers
            .replace(Regex("""Authorization:\s*Bearer\s+[\w\-_.]+"""), "Authorization: Bearer REDACTED")
            .replace(Regex("""Authorization"?\s*:\s*"Bearer\s+[\w\-_.]+""""), "Authorization\":\"Bearer REDACTED\"")
            // Passwords
            .replace(Regex("""password"?\s*[:=]\s*"[^"]+""""), "password\":\"REDACTED\"")
            .replace(Regex("""password"?\s*[:=]\s*\S+"""), "password=REDACTED")
            // Certificate fingerprints and keys
            .replace(Regex("""cert_fingerprint=SHA256:[a-f0-9]+"""), "cert_fingerprint=SHA256:REDACTED")
            .replace(Regex("""-----BEGIN\s+(PRIVATE\s+KEY|RSA\s+PRIVATE\s+KEY)-----[\s\S]*?-----END\s+(PRIVATE\s+KEY|RSA\s+PRIVATE\s+KEY)-----"""), "-----BEGIN PRIVATE KEY-----\nREDACTED\n-----END PRIVATE KEY-----")
            // Client and server IDs (optional - may want to keep for debugging)
            .replace(Regex("""client_id=[a-f0-9\-]+"""), "client_id=REDACTED")
            .replace(Regex("""server_id=[a-f0-9\-]+"""), "server_id=REDACTED")
    }
}
