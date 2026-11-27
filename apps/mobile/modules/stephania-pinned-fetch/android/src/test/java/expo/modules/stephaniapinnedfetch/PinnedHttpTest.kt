package expo.modules.stephaniapinnedfetch

import java.io.File
import java.io.FileInputStream
import java.net.InetAddress
import java.security.KeyStore
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLHandshakeException
import javax.net.ssl.SSLServerSocket
import kotlin.concurrent.thread
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test

/**
 * Against a real TLS server using certificates issued by the device PKI
 * (stephania-pki, see ci/mobile-android.sh): STEPHANIA_PKI_P12 (server key + chain),
 * STEPHANIA_PKI_CA_SHA256 (the enrolled pin).
 */
class PinnedHttpTest {
  private lateinit var server: SSLServerSocket
  private lateinit var pin: String
  private var port = 0

  /** Minimal HTTPS/1.1 responder (android.jar has no com.sun.net.httpserver). */
  private fun serve(socket: SSLServerSocket) = thread(isDaemon = true) {
    while (!socket.isClosed) {
      val client = runCatching { socket.accept() }.getOrNull() ?: break
      thread(isDaemon = true) {
        client.use { c ->
          runCatching {
            val input = c.getInputStream().bufferedReader(Charsets.ISO_8859_1)
            val requestLine = input.readLine() ?: return@runCatching
            var auth = ""
            while (true) {
              val line = input.readLine() ?: break
              if (line.isEmpty()) break
              if (line.lowercase().startsWith("authorization:")) auth = line.substringAfter(":").trim()
            }
            val body = """{"auth":"$auth","method":"${requestLine.substringBefore(' ')}"}""".toByteArray()
            val out = c.getOutputStream()
            out.write("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: ${body.size}\r\nConnection: close\r\n\r\n".toByteArray())
            out.write(body)
            out.flush()
          }
        }
      }
    }
  }

  @Before
  fun start() {
    val p12 = System.getenv("STEPHANIA_PKI_P12")
    assumeTrue("STEPHANIA_PKI_P12 not set", p12 != null && File(p12).isFile)
    pin = System.getenv("STEPHANIA_PKI_CA_SHA256")!!
    val ks = KeyStore.getInstance("PKCS12").apply { FileInputStream(p12).use { load(it, "stephania".toCharArray()) } }
    val kmf = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm()).apply { init(ks, "stephania".toCharArray()) }
    val ctx = SSLContext.getInstance("TLS").apply { init(kmf.keyManagers, null, null) }
    server = ctx.serverSocketFactory.createServerSocket(0, 16, InetAddress.getByName("127.0.0.1")) as SSLServerSocket
    port = server.localPort
    serve(server)
  }

  @After
  fun stop() {
    if (::server.isInitialized) server.close()
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
