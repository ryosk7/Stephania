import CryptoKit
import Foundation
import Security

/// Trust evaluation for a Stephania device on the LAN: the device CA enrolled over BLE
/// (management.enroll, pin = SHA-256 of its DER) is the *only* anchor.
/// The server sends leaf + CA; the CA must hash to the pin and the leaf must
/// validate with it alone — signatures, dates and host name included.
/// System roots are never consulted.
public enum PinnedTrust {
  public static func sha256Hex(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
  }

  public static func validPin(_ pin: String) -> Bool {
    pin.count == 64 && pin.allSatisfy { ("0"..."9").contains($0) || ("a"..."f").contains($0) }
  }

  /// - Parameter date: evaluation time (tests pin it; production passes nil).
  public static func evaluate(_ trust: SecTrust, host: String, pin: String, date: Date? = nil) -> Bool {
    guard validPin(pin), let chain = SecTrustCopyCertificateChain(trust) as? [SecCertificate] else { return false }
    guard let anchor = chain.first(where: { sha256Hex(SecCertificateCopyData($0) as Data) == pin }) else { return false }
    guard SecTrustSetPolicies(trust, SecPolicyCreateSSL(true, host as CFString)) == errSecSuccess,
          SecTrustSetAnchorCertificates(trust, [anchor] as CFArray) == errSecSuccess,
          SecTrustSetAnchorCertificatesOnly(trust, true) == errSecSuccess else { return false }
    if let date, SecTrustSetVerifyDate(trust, date as CFDate) != errSecSuccess { return false }
    // Local devices publish no revocation information; never go online for it.
    _ = SecTrustSetNetworkFetchAllowed(trust, false)
    var error: CFError?
    return SecTrustEvaluateWithError(trust, &error)
  }

  /// Build a trust object from DER certificates, leaf first (tests, tools).
  public static func trust(chain: [Data]) -> SecTrust? {
    let certs = chain.compactMap { SecCertificateCreateWithData(nil, $0 as CFData) }
    guard certs.count == chain.count, !certs.isEmpty else { return nil }
    var trust: SecTrust?
    guard SecTrustCreateWithCertificates(certs as CFArray, SecPolicyCreateBasicX509(), &trust) == errSecSuccess else { return nil }
    return trust
  }
}

/// URLSession delegate applying [`PinnedTrust`] and refusing redirects.
public final class PinnedSessionDelegate: NSObject, URLSessionTaskDelegate {
  let pin: String

  public init(pin: String) { self.pin = pin }

  public func urlSession(_ session: URLSession, didReceive challenge: URLAuthenticationChallenge,
                         completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
    guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
          let trust = challenge.protectionSpace.serverTrust,
          PinnedTrust.evaluate(trust, host: challenge.protectionSpace.host, pin: pin) else {
      completionHandler(.cancelAuthenticationChallenge, nil)
      return
    }
    completionHandler(.useCredential, URLCredential(trust: trust))
  }

  public func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                         newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
    completionHandler(nil)
  }
}
