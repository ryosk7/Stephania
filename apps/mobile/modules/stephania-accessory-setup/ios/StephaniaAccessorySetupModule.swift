import CoreBluetooth
import ExpoModulesCore
import UIKit
#if canImport(AccessorySetupKit)
import AccessorySetupKit
#endif

public class StephaniaAccessorySetupModule: Module {
  public func definition() -> ModuleDefinition {
    Name("StephaniaAccessorySetup")

    Function("isSupported") { () -> Bool in
      #if canImport(AccessorySetupKit)
      if #available(iOS 18.0, *) { return true }
      #endif
      return false
    }

    AsyncFunction("pick") { (serviceUuid: String, name: String, promise: Promise) in
      #if canImport(AccessorySetupKit)
      if #available(iOS 18.0, *) {
        AccessoryPicker.shared.pick(serviceUuid: serviceUuid, name: name, promise: promise)
        return
      }
      #endif
      promise.resolve(nil)
    }
    .runOnQueue(.main)
  }
}

#if canImport(AccessorySetupKit)
/// One session for the app's lifetime. The picker is shown once the session
/// reports `.activated`; the result is the accessory added before the picker
/// is dismissed, or nil when the person closes it.
@available(iOS 18.0, *)
final class AccessoryPicker {
  static let shared = AccessoryPicker()
  private let session = ASAccessorySession()
  private var activated = false
  private var activating = false
  private var waiting: (() -> Void)?
  private var pending: Promise?
  private var added: ASAccessory?

  func pick(serviceUuid: String, name: String, promise: Promise) {
    if pending != nil {
      promise.reject("busy", "The setup sheet is already open.")
      return
    }
    pending = promise
    added = nil
    let show = { [weak self] in
      guard let self else { return }
      let descriptor = ASDiscoveryDescriptor()
      descriptor.bluetoothServiceUUID = CBUUID(string: serviceUuid)
      let item = ASPickerDisplayItem(name: name, productImage: Self.productImage(), descriptor: descriptor)
      self.session.showPicker(for: [item]) { error in
        if let error { self.finish(error: error) }
      }
    }
    if activated {
      show()
    } else {
      waiting = show
      if !activating {
        activating = true
        session.activate(on: .main) { [weak self] event in self?.handle(event) }
      }
    }
  }

  private func handle(_ event: ASAccessoryEvent) {
    switch event.eventType {
    case .activated:
      activated = true
      waiting?()
      waiting = nil
    case .accessoryAdded:
      added = event.accessory
    case .pickerDidDismiss:
      finish(error: nil)
    case .invalidated:
      activated = false
      activating = false
      finish(error: nil)
    default:
      break
    }
  }

  private func finish(error: Error?) {
    guard let promise = pending else { return }
    pending = nil
    if let error, (error as? ASError)?.code != .userCancelled {
      promise.reject("picker_failed", error.localizedDescription)
      return
    }
    guard let accessory = added, let id = accessory.bluetoothIdentifier else {
      promise.resolve(nil)
      return
    }
    promise.resolve(["id": id.uuidString, "name": accessory.displayName])
  }

  /// The device as drawn on the setup sheet (DESIGN.md §6, DeviceArt in
  /// the app): a Raspberry Pi 5 from above in its real colours, drawn on the
  /// same 120 × 86 grid at twice the size.
  static func productImage() -> UIImage {
    func c(_ hex: UInt32) -> UIColor {
      UIColor(red: CGFloat(hex >> 16 & 0xFF) / 255, green: CGFloat(hex >> 8 & 0xFF) / 255, blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
    }
    func box(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat, _ r: CGFloat, _ color: UInt32) {
      c(color).setFill()
      UIBezierPath(roundedRect: CGRect(x: x, y: y, width: w, height: h), cornerRadius: r).fill()
    }
    func dot(_ x: CGFloat, _ y: CGFloat, _ r: CGFloat, _ color: UInt32) {
      c(color).setFill()
      UIBezierPath(ovalIn: CGRect(x: x - r, y: y - r, width: r * 2, height: r * 2)).fill()
    }
    return UIGraphicsImageRenderer(size: CGSize(width: 240, height: 172)).image { ctx in
      ctx.cgContext.scaleBy(x: 2, y: 2)
      box(4, 8, 102, 70, 7, 0x1F6B3F)
      for (x, y) in [(10.5, 14.5), (68.5, 14.5), (10.5, 71.5), (68.5, 71.5)] as [(CGFloat, CGFloat)] {
        dot(x, y, 3.0, 0xD4AF37); dot(x, y, 1.8, 0x0E3A21)
      }
      box(17, 11, 46, 7, 1.5, 0x18181B)
      for row in [CGFloat(12.3), 15.3] {
        var x: CGFloat = 19.5
        while x < 61 { box(x, row, 1.4, 1.4, 0.3, 0xD4AF37); x += 2.3 }
      }
      box(18, 26, 34, 34, 5, 0xB9BEC4)
      dot(35, 43, 12, 0x27272A)
      c(0x52525B).setStroke()
      let blades = UIBezierPath()
      for k in 0..<8 {
        let a = CGFloat(k) * .pi / 4
        blades.move(to: CGPoint(x: 35 + 4.5 * cos(a), y: 43 + 4.5 * sin(a)))
        blades.addLine(to: CGPoint(x: 35 + 9.5 * cos(a), y: 43 + 9.5 * sin(a)))
      }
      blades.lineWidth = 1.6
      blades.lineCapStyle = .round
      blades.stroke()
      dot(35, 43, 3, 0x52525B)
      box(58, 30, 16, 16, 2, 0x27272A)
      box(94, 12, 20, 15, 1.5, 0xC4C8CD); box(99, 17, 12, 2.5, 0.6, 0x18181B)
      box(94, 31, 20, 15, 1.5, 0xC4C8CD); box(99, 36, 12, 2.5, 0.6, 0x2563EB)
      box(92, 51, 22, 20, 1.5, 0xC4C8CD); box(98, 56, 12, 10, 1, 0x3F3F46)
      box(16, 74, 10, 6, 2, 0xC4C8CD); box(32, 74, 9, 6, 1.5, 0xC4C8CD); box(46, 74, 9, 6, 1.5, 0xC4C8CD)
      dot(84, 70, 2.6, 0xF04438)
    }
  }
}
#endif
