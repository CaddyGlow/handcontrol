package com.handcontrol.core.security

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.firstOrNull
import kotlinx.coroutines.flow.map
import timber.log.Timber
import java.io.ByteArrayOutputStream
import java.math.BigInteger
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.cert.X509Certificate
import java.util.Date
import javax.inject.Inject
import javax.inject.Singleton
import javax.security.auth.x500.X500Principal

private val Context.securityDataStore: DataStore<Preferences> by preferencesDataStore(
    name = "security_prefs"
)

@Singleton
class AndroidKeystoreCertificateManager @Inject constructor(
    @ApplicationContext private val context: Context
) : ClientCertificateManager {

    private val keyStore: KeyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply {
        load(null)
    }

    override suspend fun loadOrCreate(): ClientCertificate {
        if (keyStore.containsAlias(CLIENT_KEY_ALIAS)) {
            Timber.d("Loading existing client certificate")
            val certificate = keyStore.getCertificate(CLIENT_KEY_ALIAS) as X509Certificate
            return ClientCertificate(
                certificateDer = certificate.encoded,
                privateKeyAlias = CLIENT_KEY_ALIAS
            )
        }

        Timber.i("Generating new client certificate")
        return generateClientCertificate()
    }

    override suspend fun pinServerFingerprint(fingerprint: String) {
        Timber.i("Pinning server certificate fingerprint")
        context.securityDataStore.edit { prefs ->
            prefs[SERVER_FINGERPRINT_KEY] = fingerprint
        }
    }

    override suspend fun getPinnedServerFingerprint(): String? {
        return context.securityDataStore.data
            .map { prefs -> prefs[SERVER_FINGERPRINT_KEY] }
            .firstOrNull()
    }

    override suspend fun clearPinnedServer() {
        Timber.i("Clearing pinned server certificate")
        context.securityDataStore.edit { prefs ->
            prefs.remove(SERVER_FINGERPRINT_KEY)
        }
    }

    private fun generateClientCertificate(): ClientCertificate {
        val keyPairGenerator = KeyPairGenerator.getInstance(
            KeyProperties.KEY_ALGORITHM_EC,
            ANDROID_KEYSTORE
        )

        val spec = KeyGenParameterSpec.Builder(
            CLIENT_KEY_ALIAS,
            KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY
        )
            .setAlgorithmParameterSpec(java.security.spec.ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .setUserAuthenticationRequired(false)
            .setCertificateSubject(X500Principal("CN=HandControl Client"))
            .setCertificateSerialNumber(BigInteger.valueOf(System.currentTimeMillis()))
            .setCertificateNotBefore(Date())
            .setCertificateNotAfter(Date(System.currentTimeMillis() + CERT_VALIDITY_MS))
            .build()

        keyPairGenerator.initialize(spec)
        keyPairGenerator.generateKeyPair()

        val certificate = keyStore.getCertificate(CLIENT_KEY_ALIAS) as X509Certificate

        return ClientCertificate(
            certificateDer = certificate.encoded,
            privateKeyAlias = CLIENT_KEY_ALIAS
        )
    }

    companion object {
        private const val ANDROID_KEYSTORE = "AndroidKeyStore"
        private const val CLIENT_KEY_ALIAS = "handcontrol_client_key"
        private val SERVER_FINGERPRINT_KEY = stringPreferencesKey("server_fingerprint")
        private const val CERT_VALIDITY_MS = 10L * 365 * 24 * 60 * 60 * 1000 // 10 years
    }
}
