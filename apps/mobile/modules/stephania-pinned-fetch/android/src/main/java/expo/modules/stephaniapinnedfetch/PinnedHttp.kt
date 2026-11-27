package expo.modules.stephaniapinnedfetch

import java.io.ByteArrayOutputStream
import java.net.URL
import java.security.KeyStore
import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLContext
import javax.net.ssl.TrustManagerFactory
import javax.net.ssl.X509TrustManager

/**
 * HTTPS to a Stephania device on the LAN, trusting only the device CA enrolled over
 * BLE (management.enroll). The server sends leaf + CA; the CA in the chain
 * must hash (SHA-256 of DER) to the enrolled pin, and the leaf must then
 * validate with that CA as the *only* anchor — dates, signatures and the
 * host name included. System roots are never consulted.
 */
object PinnedTrust {
  fun sha256Hex(bytes: ByteArray): String =
    MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

  class Manager(private val pin: String) : X509TrustManager {
    init {
      require(pin.matches(Regex("^[0-9a-f]{64}$"))) { "pin must be 64 lowercase hex digits" }
    }

    override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
      if (chain.isEmpty()) throw CertificateException("empty chain")
      val anchor = chain.firstOrNull { sha256Hex(it.encoded) == pin }
        ?: throw CertificateException("no certificate in the chain matches the enrolled device CA")
      val store = KeyStore.getInstance(KeyStore.getDefaultType()).apply { load(null, null); setCertificateEntry("stephania-ca", anchor) }
      val factory = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm()).apply { init(store) }
      val delegate = factory.trustManagers.filterIsInstance<X509TrustManager>().first()
      delegate.checkServerTrusted(chain, authType)
    }

    override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) =
      throw CertificateException("client certificates are not used")

    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
  }

  fun context(pin: String): SSLContext =
    SSLContext.getInstance("TLS").apply { init(null, arrayOf(Manager(pin)), null) }
}

data class PinnedResponse(val status: Int, val headers: Map<String, String>, val body: String)

object PinnedHttp {
  private const val MAX_BODY = 1 shl 20

  fun request(url: String, method: String, headers: Map<String, String>, body: String?, pin: String, timeoutMs: Int): PinnedResponse {
    val u = URL(url)
    require(u.protocol == "https") { "only https is allowed" }
    require(method in setOf("GET", "POST", "PUT", "PATCH", "DELETE")) { "unsupported method" }
    // Never through a proxy: the device is on the local network.
    val conn = u.openConnection(java.net.Proxy.NO_PROXY) as HttpsURLConnection
    conn.sslSocketFactory = PinnedTrust.context(pin).socketFactory
    // Host name is checked by the platform verifier against the leaf SAN.
    conn.hostnameVerifier = HttpsURLConnection.getDefaultHostnameVerifier()
    conn.requestMethod = method
    conn.connectTimeout = timeoutMs
    conn.readTimeout = timeoutMs
    conn.instanceFollowRedirects = false
    conn.useCaches = false
    headers.forEach { (k, v) -> conn.setRequestProperty(k, v) }
    try {
      if (body != null) {
        conn.doOutput = true
        conn.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }
      }
      val status = conn.responseCode
      val stream = if (status >= 400) conn.errorStream else conn.inputStream
      val out = ByteArrayOutputStream()
      stream?.use { s ->
        val buf = ByteArray(8192)
        while (true) {
          val n = s.read(buf)
          if (n < 0) break
          if (out.size() + n > MAX_BODY) throw IllegalStateException("response larger than $MAX_BODY bytes")
          out.write(buf, 0, n)
        }
      }
      val h = conn.headerFields.filterKeys { it != null }.mapKeys { it.key.lowercase() }.mapValues { it.value.joinToString(", ") }
      return PinnedResponse(status, h, out.toString("UTF-8"))
    } finally {
      conn.disconnect()
    }
  }
}
