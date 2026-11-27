package expo.modules.stephaniapinnedfetch

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import org.json.JSONObject

class StephaniaPinnedFetchModule : Module() {
  override fun definition() = ModuleDefinition {
    Name("StephaniaPinnedFetch")

    // Runs off the JS thread (AsyncFunction default); the call blocks only
    // that background thread.
    AsyncFunction("request") { url: String, method: String, headersJson: String, body: String?, caSha256: String, timeoutMs: Int ->
      val json = JSONObject(headersJson)
      val headers = json.keys().asSequence().associateWith { json.getString(it) }
      val r = PinnedHttp.request(url, method, headers, body, caSha256, timeoutMs)
      mapOf("status" to r.status, "headers" to JSONObject(r.headers as Map<*, *>).toString(), "body" to r.body)
    }
  }
}
