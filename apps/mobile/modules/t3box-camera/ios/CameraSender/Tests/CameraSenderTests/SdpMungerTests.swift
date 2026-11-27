import XCTest
@testable import CameraSender

final class SdpMungerTests: XCTestCase {
  // Representative libwebrtc video section before restriction: VP8, VP9,
  // H.264 constrained high and constrained baseline (pm 1 and pm 0), each
  // with RTX, plus RED/ULPFEC.
  static let offer = """
  v=0
  o=- 1 2 IN IP4 127.0.0.1
  s=-
  t=0 0
  a=group:BUNDLE 0
  m=video 9 UDP/TLS/RTP/SAVPF 96 97 98 99 100 101 102 103 104 105 106 107 108
  c=IN IP4 0.0.0.0
  a=mid:0
  a=sendonly
  a=msid:t3box rear-1
  a=rtcp-fb:* ccm fir
  a=rtpmap:96 VP8/90000
  a=rtcp-fb:96 nack pli
  a=rtpmap:97 rtx/90000
  a=fmtp:97 apt=96
  a=rtpmap:98 VP9/90000
  a=fmtp:98 profile-id=0
  a=rtpmap:99 rtx/90000
  a=fmtp:99 apt=98
  a=rtpmap:100 H264/90000
  a=rtcp-fb:100 nack pli
  a=fmtp:100 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640c1f
  a=rtpmap:101 rtx/90000
  a=fmtp:101 apt=100
  a=rtpmap:102 H264/90000
  a=rtcp-fb:102 nack
  a=rtcp-fb:102 nack pli
  a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f
  a=rtpmap:103 rtx/90000
  a=fmtp:103 apt=102
  a=rtpmap:104 H264/90000
  a=fmtp:104 level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f
  a=rtpmap:105 rtx/90000
  a=fmtp:105 apt=104
  a=rtpmap:106 red/90000
  a=rtpmap:107 rtx/90000
  a=fmtp:107 apt=106
  a=rtpmap:108 ulpfec/90000
  a=ssrc:1 cname:x
  """

  func testKeepsOnlyBaselineH264AndItsRtx() throws {
    let out = try SdpMunger.restrictToBaselineH264(Self.offer)
    let lines = out.components(separatedBy: "\r\n")
    XCTAssertTrue(lines.contains("m=video 9 UDP/TLS/RTP/SAVPF 102 103"))
    let rtp = lines.filter { $0.hasPrefix("a=rtpmap:") || $0.hasPrefix("a=fmtp:") || $0.hasPrefix("a=rtcp-fb:") }
    XCTAssertEqual(rtp, [
      "a=rtcp-fb:* ccm fir",
      "a=rtpmap:102 H264/90000",
      "a=rtcp-fb:102 nack",
      "a=rtcp-fb:102 nack pli",
      "a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
      "a=rtpmap:103 rtx/90000",
      "a=fmtp:103 apt=102",
    ])
    // Non-codec attributes survive in order.
    XCTAssertTrue(lines.contains("a=msid:t3box rear-1"))
    XCTAssertTrue(lines.contains("a=ssrc:1 cname:x"))
    XCTAssertTrue(out.hasSuffix("\r\n"))
  }

  func testRefusesAnOfferWithoutBaselineH264() {
    let vp8Only = Self.offer.replacingOccurrences(of: "profile-level-id=42e01f", with: "profile-level-id=4d001f")
    XCTAssertThrowsError(try SdpMunger.restrictToBaselineH264(vp8Only)) { e in
      XCTAssertEqual(e as? SdpMunger.Failure, .noBaselineH264(mid: "0"))
    }
  }

  func testLeavesNonVideoSectionsAlone() throws {
    let withAudio = Self.offer + "\nm=audio 9 UDP/TLS/RTP/SAVPF 111\na=rtpmap:111 opus/48000/2\na=mid:1"
    let out = try SdpMunger.restrictToBaselineH264(withAudio)
    XCTAssertTrue(out.contains("m=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2"))
  }
}
