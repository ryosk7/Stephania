package expo.modules.t3boxpinnedfetch

import com.sun.net.httpserver.HttpsConfigurator
import com.sun.net.httpserver.HttpsServer
import java.io.File
import java.io.FileInputStream
import java.net.InetSocketAddress
import java.security.KeyStore
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLHandshakeException
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test

/**
 * Against a real TLS server using certificates issued by the device PKI
 * (t3box-pki, see ci/mobile-android.sh): T3BOX_PKI_P12 (server key + chain),
 * T3BOX_PKI_CA_SHA256 (the enrolled pin).
 */
class PinnedHttpTest {
  private lateinit var server: HttpsServer
  private lateinit var pin: String
  private var port = 0

  @Before
  fun start() {
    val p12 = System.getenv("T3BOX_PKI_P12")
    assumeTrue("T3BOX_PKI_P12 not set", p12 != null && File(p12).isFile)
    pin = System.getenv("T3BOX_PKI_CA_SHA256")!!
    val ks = KeyStore.getInstance("PKCS12").apply { FileInputStream(p12).use { load(it, "t3box".toCharArray()) } }
    val kmf = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply { init(ks, "t3box".toCharArray()) }
    val ctx = SSLContext.getInstance("TLS").apply { init(kmf.keyManagers, null, null) }
    server = HttpsServer.create(InetSocketAddress("127.0.0.1", 0), 0)
    server.httpsConfigurator = HttpsConfigurator(ctx)
    server.createContext("/api/v1/status") { ex ->
      val auth = ex.requestHeaders.getFirst("Authorization") ?: ""
      val body = """{"auth":"$auth","method":"${ex.requestMethod}"}""".toByteArray()
      ex.responseHeaders.add("Content-Type", "application/json")
      ex.sendResponseHeaders(200, body.size.toLong())
      ex.responseBody.use { it.write(body) }
    }
    server.start()
    port = server.address.port
  }

  @After
  fun stop() {
    if (::server.isInitialized) server.stop(0)
  }

  @Test
  fun acceptsTheEnrolledCaAndSendsHeaders() {
    val r = PinnedHttp.request("https://127.0.0.1:$port/api/v1/status", "GET", mapOf("Authorization" to "Bearer t3b_x"), null, pin, 5000)
    assertEquals(200, r.status)
    assertEquals("""{"auth":"Bearer t3b_x","method":"GET"}""", r.body)
    assertEquals("application/json", r.headers["content-type"])
  }

  @Test
  fun refusesAnyOtherPin() {
    val other = "0".repeat(64)
    assertThrows(SSLHandshakeException::class.java) {
      PinnedHttp.request("https://127.0.0.1:$port/api/v1/status", "GET", emptyMap(), null, other, 5000)
    }
  }

  @Test
  fun refusesANameTheCertificateDoesNotCover() {
    // "localhost" is not in the device certificate (only <host>.local and its addresses).
    val e = runCatching { PinnedHttp.request("https://localhost:$port/api/v1/status", "GET", emptyMap(), null, pin, 5000) }.exceptionOrNull()
    assertEquals(true, e is javax.net.ssl.SSLPeerUnverifiedException || e is SSLHandshakeException || e?.cause is java.security.cert.CertificateException)
  }

  @Test
  fun refusesPlainHttpAndBadPins() {
    assertThrows(IllegalArgumentException::class.java) { PinnedHttp.request("http://127.0.0.1:$port/", "GET", emptyMap(), null, pin, 5000) }
    assertThrows(IllegalArgumentException::class.java) { PinnedTrust.Manager("ABC") }
  }
}
