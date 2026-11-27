import ExpoModulesCore
import Foundation

/// HTTPS to the Stephania management API trusting only the enrolled device CA
/// (PinnedTrust). No cookies, no cache, no redirects, no system proxy.
public class StephaniaPinnedFetchModule: Module {
  static let maxBody = 1 << 20

  public func definition() -> ModuleDefinition {
    Name("StephaniaPinnedFetch")

    AsyncFunction("request") { (url: String, method: String, headersJson: String, body: String?, caSha256: String, timeoutMs: Int, promise: Promise) in
      guard let u = URL(string: url), u.scheme == "https" else {
        promise.reject("E_PINNED_URL", "only https URLs are allowed")
        return
      }
      guard ["GET", "POST", "PUT", "PATCH", "DELETE"].contains(method), PinnedTrust.validPin(caSha256) else {
        promise.reject("E_PINNED_ARGS", "unsupported method or invalid pin")
        return
      }
      var request = URLRequest(url: u, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: Double(timeoutMs) / 1000)
      request.httpMethod = method
      request.httpShouldHandleCookies = false
      if let data = headersJson.data(using: .utf8), let headers = try? JSONSerialization.jsonObject(with: data) as? [String: String] {
        for (k, v) in headers { request.setValue(v, forHTTPHeaderField: k) }
      }
      if let body { request.httpBody = body.data(using: .utf8) }
      let config = URLSessionConfiguration.ephemeral
      config.connectionProxyDictionary = [:]
      config.urlCache = nil
      let session = URLSession(configuration: config, delegate: PinnedSessionDelegate(pin: caSha256), delegateQueue: nil)
      session.dataTask(with: request) { data, response, error in
        defer { session.finishTasksAndInvalidate() }
        if let error {
          promise.reject("E_PINNED_REQUEST", error.localizedDescription)
          return
        }
        guard let http = response as? HTTPURLResponse else {
          promise.reject("E_PINNED_REQUEST", "no HTTP response")
          return
        }
        let bytes = data ?? Data()
        if bytes.count > Self.maxBody {
          promise.reject("E_PINNED_REQUEST", "response too large")
          return
        }
        var headers: [String: String] = [:]
        for (k, v) in http.allHeaderFields { headers[String(describing: k).lowercased()] = String(describing: v) }
        let headerJson = (try? JSONSerialization.data(withJSONObject: headers)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
        promise.resolve(["status": http.statusCode, "headers": headerJson, "body": String(decoding: bytes, as: UTF8.self)])
      }.resume()
    }
  }
}
