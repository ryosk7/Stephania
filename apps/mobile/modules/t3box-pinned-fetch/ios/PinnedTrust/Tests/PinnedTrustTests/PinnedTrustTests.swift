import Foundation
import PinnedTrust
import XCTest

/// Fixtures are real device-PKI output (t3box-pki, see Fixtures/README):
/// a CA name-constrained to t3box-ci.local + private addresses, and a
/// server certificate for t3box-ci.local and 127.0.0.1; plus an unrelated
/// second device's CA and certificate.
final class PinnedTrustTests: XCTestCase {
  func der(_ name: String) throws -> Data {
    let url = try XCTUnwrap(Bundle.module.url(forResource: name, withExtension: "der", subdirectory: "Fixtures"))
    return try Data(contentsOf: url)
  }

  var at: Date { Date(timeIntervalSince1970: 1_800_000_000) } // within the fixtures' validity

  func testAcceptsTheEnrolledCaForCertifiedNamesAndAddresses() throws {
    let (leaf, ca) = (try der("leaf"), try der("ca"))
    let pin = PinnedTrust.sha256Hex(ca)
    for host in ["t3box-ci.local", "127.0.0.1"] {
      let trust = try XCTUnwrap(PinnedTrust.trust(chain: [leaf, ca]))
      XCTAssertTrue(PinnedTrust.evaluate(trust, host: host, pin: pin, date: at), host)
    }
  }

  func testRefusesOtherHostsPinsAndForeignChains() throws {
    let (leaf, ca, otherLeaf, otherCa) = (try der("leaf"), try der("ca"), try der("other-leaf"), try der("other-ca"))
    let pin = PinnedTrust.sha256Hex(ca)
    func eval(_ chain: [Data], _ host: String, _ p: String) throws -> Bool {
      PinnedTrust.evaluate(try XCTUnwrap(PinnedTrust.trust(chain: chain)), host: host, pin: p, date: at)
    }
    XCTAssertFalse(try eval([leaf, ca], "localhost", pin), "name not in the certificate")
    XCTAssertFalse(try eval([leaf, ca], "evil.example", pin))
    XCTAssertFalse(try eval([leaf, ca], "t3box-ci.local", String(repeating: "0", count: 64)), "wrong pin")
    XCTAssertFalse(try eval([leaf, ca], "t3box-ci.local", "NOT-A-PIN"))
    // Another device's server presenting our CA next to its own leaf.
    XCTAssertFalse(try eval([otherLeaf, ca], "t3box-ci.local", pin), "leaf not issued by the pinned CA")
    // The other device's own chain is fine only under its own pin.
    XCTAssertTrue(try eval([otherLeaf, otherCa], "t3box-ci.local", PinnedTrust.sha256Hex(otherCa)))
    XCTAssertFalse(try eval([otherLeaf, otherCa], "t3box-ci.local", pin))
  }

  func testExpiredCertificatesAreRefused() throws {
    let (leaf, ca) = (try der("leaf"), try der("ca"))
    let trust = try XCTUnwrap(PinnedTrust.trust(chain: [leaf, ca]))
    XCTAssertFalse(PinnedTrust.evaluate(trust, host: "t3box-ci.local", pin: PinnedTrust.sha256Hex(ca), date: Date(timeIntervalSince1970: 4_000_000_000)))
  }
}
