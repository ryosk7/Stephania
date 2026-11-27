import Foundation

/// Restricts a libwebrtc offer to what t3box-phone decodes (docs/PHONE.md):
/// H.264 constrained baseline (`profile-level-id=42e0xx`, e.g. 42e01f) with
/// `packetization-mode=1`, plus its RTX payload. Every other video payload
/// (VP8, VP9, AV1, other H.264 profiles, RED/ULPFEC) is removed from each
/// video m-line together with its rtpmap/fmtp/rtcp-fb attributes.
///
/// Done on the local offer before `setLocalDescription`, which libwebrtc
/// accepts for codec removal; this does not depend on the per-version
/// `setCodecPreferences` API surface.
public enum SdpMunger {
  public enum Failure: Error, Equatable, CustomStringConvertible {
    /// `offered`: the H.264 fmtp lines that were present, for diagnosis.
    case noBaselineH264(mid: String, offered: [String])

    public var description: String {
      switch self {
      case let .noBaselineH264(mid, offered):
        return "video section \(mid) has no H.264 constrained baseline with packetization-mode=1 (H.264 offered: \(offered.isEmpty ? "none" : offered.joined(separator: " | ")))"
      }
    }
  }

  /// Constrained baseline at any level: profile_idc 0x42 with
  /// constraint_set1 (0xe0 profile-iop). t3box-phone (webrtc-rs) matches
  /// H.264 on profile and packetization mode, not level, and the Pi's
  /// decoder accepts every level a phone sends.
  public static func isConstrainedBaseline(_ profileLevelId: String) -> Bool {
    let p = profileLevelId.lowercased()
    return p.count == 6 && p.hasPrefix("42") && (UInt8(p.dropFirst(2).prefix(2), radix: 16).map { $0 & 0x40 != 0 } ?? false)
  }

  public static func restrictToBaselineH264(_ sdp: String) throws -> String {
    let lines = sdp.components(separatedBy: "\r\n").flatMap { $0.components(separatedBy: "\n") }.filter { !$0.isEmpty }
    var out: [String] = []
    var section: [String] = []

    func flush() throws {
      guard let first = section.first else { return }
      if first.hasPrefix("m=video ") { out += try rewriteVideo(section) } else { out += section }
      section = []
    }
    for line in lines {
      if line.hasPrefix("m=") { try flush() }
      section.append(line)
    }
    try flush()
    return out.joined(separator: "\r\n") + "\r\n"
  }

  /// Payload types of baseline H.264 (packetization-mode 1) in a media section, in m-line order.
  public static func baselinePayloads(_ section: [String]) -> [String] {
    var h264 = Set<String>()
    var ok = Set<String>()
    for l in section {
      if let (pt, rest) = attribute(l, "a=rtpmap:"), rest.uppercased() == "H264/90000" { h264.insert(pt) }
    }
    for l in section {
      guard let (pt, rest) = attribute(l, "a=fmtp:"), h264.contains(pt) else { continue }
      var params: [String: String] = [:]
      for p in rest.split(separator: ";") {
        let kv = p.split(separator: "=", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
        if kv.count == 2 { params[kv[0]] = kv[1] }
      }
      if let plid = params["profile-level-id"], isConstrainedBaseline(plid), params["packetization-mode"] == "1" { ok.insert(pt) }
    }
    let order = section[0].split(separator: " ").dropFirst(3).map(String.init)
    return order.filter { ok.contains($0) }
  }

  static func rewriteVideo(_ section: [String]) throws -> [String] {
    let mid = section.first { $0.hasPrefix("a=mid:") }.map { String($0.dropFirst(6)) } ?? "?"
    guard let keep = baselinePayloads(section).first else {
      let h264 = Set(section.compactMap { l in attribute(l, "a=rtpmap:").flatMap { $0.1.uppercased() == "H264/90000" ? $0.0 : nil } })
      throw Failure.noBaselineH264(mid: mid, offered: section.filter { l in attribute(l, "a=fmtp:").map { h264.contains($0.0) } ?? false })
    }
    var allowed: Set<String> = [keep]
    for l in section {
      if let (pt, rest) = attribute(l, "a=fmtp:"), rest == "apt=\(keep)" { allowed.insert(pt) }
    }
    var mline = section[0].split(separator: " ").map(String.init)
    let payloads = mline.dropFirst(3).filter { allowed.contains($0) }
    mline = Array(mline.prefix(3)) + payloads
    var result = [mline.joined(separator: " ")]
    for l in section.dropFirst() {
      if let (pt, _) = attribute(l, "a=rtpmap:") ?? attribute(l, "a=fmtp:") ?? attribute(l, "a=rtcp-fb:"), !allowed.contains(pt) { continue }
      result.append(l)
    }
    return result
  }

  /// `a=rtpmap:102 H264/90000` → ("102", "H264/90000").
  static func attribute(_ line: String, _ prefix: String) -> (String, String)? {
    guard line.hasPrefix(prefix) else { return nil }
    let body = line.dropFirst(prefix.count)
    guard let space = body.firstIndex(of: " ") else { return nil }
    let pt = String(body[..<space])
    guard pt.allSatisfy(\.isNumber) else { return nil }
    return (pt, String(body[body.index(after: space)...]))
  }
}
