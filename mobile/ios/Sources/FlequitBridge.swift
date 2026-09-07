import Foundation
import UIKit
import UniformTypeIdentifiers
import UserNotifications

/// Bridges the UIKit APIs the Rust platform layer cannot reach on its own.
///
/// `crates/flequit-platform/src/platform/ios.rs` declares each `@_cdecl`
/// function below as an `extern "C"` import. Keeping the Objective-C surface on
/// this side means Xcode type-checks it, and it is the only place that can
/// reach the key window — which a library cannot.
///
/// Every entry point is safe to call from a background thread: work that UIKit
/// requires on the main thread is dispatched there explicitly.

// MARK: - Status codes shared with Rust

private let bridgeOK: Int32 = 0
private let bridgeFailed: Int32 = 1

private let permissionGranted: Int32 = 0
private let permissionDenied: Int32 = 1
private let permissionNotDetermined: Int32 = 2

/// Matches `LifecycleEvent::from_code`.
private let lifecycleSuspend: Int32 = 0
private let lifecycleResume: Int32 = 1
private let lifecycleLowMemory: Int32 = 2

/// Implemented in Rust; see `flequit_ios_lifecycle_event`.
@_silgen_name("flequit_ios_lifecycle_event")
func flequit_ios_lifecycle_event(_ event: Int32)

/// Implemented in Rust; runs the Slint event loop and does not return.
@_silgen_name("flequit_ios_main")
func flequit_ios_main() -> Int32

// MARK: - Helpers

private func string(from pointer: UnsafePointer<CChar>?) -> String? {
    guard let pointer else { return nil }
    return String(cString: pointer)
}

/// Runs `body` on the main thread and waits for it.
///
/// UIKit presentation and `UIApplication.open` must happen on the main thread,
/// but Rust calls in from a Tokio worker. Callers must never invoke these
/// entry points *from* the main thread, or this would deadlock.
private func onMainSync<T>(_ body: @escaping () -> T) -> T {
    if Thread.isMainThread {
        return body()
    }
    var result: T!
    let semaphore = DispatchSemaphore(value: 0)
    DispatchQueue.main.async {
        result = body()
        semaphore.signal()
    }
    semaphore.wait()
    return result
}

private func topViewController() -> UIViewController? {
    let scene = UIApplication.shared.connectedScenes
        .compactMap { $0 as? UIWindowScene }
        .first { $0.activationState == .foregroundActive }
    var controller = scene?.windows.first(where: \.isKeyWindow)?.rootViewController
    while let presented = controller?.presentedViewController {
        controller = presented
    }
    return controller
}

// MARK: - Device

@_cdecl("flequit_ios_form_factor")
public func flequitIosFormFactor() -> Int32 {
    Int32(UIDevice.current.userInterfaceIdiom.rawValue)
}

// MARK: - Notifications

@_cdecl("flequit_ios_notification_permission")
public func flequitIosNotificationPermission() -> Int32 {
    let semaphore = DispatchSemaphore(value: 0)
    var result = permissionNotDetermined

    UNUserNotificationCenter.current().getNotificationSettings { settings in
        switch settings.authorizationStatus {
        case .authorized, .provisional, .ephemeral:
            result = permissionGranted
        case .denied:
            result = permissionDenied
        default:
            result = permissionNotDetermined
        }
        semaphore.signal()
    }

    semaphore.wait()
    return result
}

@_cdecl("flequit_ios_request_notification_permission")
public func flequitIosRequestNotificationPermission() -> Int32 {
    let semaphore = DispatchSemaphore(value: 0)
    var result = permissionNotDetermined

    UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) {
        granted, error in
        if let error {
            NSLog("flequit: notification authorisation failed: \(error)")
        }
        result = granted ? permissionGranted : permissionDenied
        semaphore.signal()
    }

    semaphore.wait()
    return result
}

/// Schedules a local notification. A `delaySeconds` of zero delivers at once.
@_cdecl("flequit_ios_notify")
public func flequitIosNotify(
    _ title: UnsafePointer<CChar>?,
    _ body: UnsafePointer<CChar>?,
    _ identifier: UnsafePointer<CChar>?,
    _ delaySeconds: Double
) -> Int32 {
    guard let identifier = string(from: identifier) else { return bridgeFailed }

    let content = UNMutableNotificationContent()
    content.title = string(from: title) ?? ""
    content.body = string(from: body) ?? ""
    content.sound = .default

    // UNTimeIntervalNotificationTrigger rejects intervals below one second, so
    // an immediate notification uses no trigger at all.
    let trigger: UNNotificationTrigger? =
        delaySeconds >= 1
        ? UNTimeIntervalNotificationTrigger(timeInterval: delaySeconds, repeats: false)
        : nil

    let request = UNNotificationRequest(
        identifier: identifier, content: content, trigger: trigger)

    // Reusing the identifier replaces the pending request, which is what makes
    // rescheduling a reminder idempotent.
    let semaphore = DispatchSemaphore(value: 0)
    var result = bridgeOK
    UNUserNotificationCenter.current().add(request) { error in
        if let error {
            NSLog("flequit: could not add notification: \(error)")
            result = bridgeFailed
        }
        semaphore.signal()
    }
    semaphore.wait()
    return result
}

@_cdecl("flequit_ios_cancel_notification")
public func flequitIosCancelNotification(_ identifier: UnsafePointer<CChar>?) -> Int32 {
    guard let identifier = string(from: identifier) else { return bridgeFailed }
    let center = UNUserNotificationCenter.current()
    center.removePendingNotificationRequests(withIdentifiers: [identifier])
    center.removeDeliveredNotifications(withIdentifiers: [identifier])
    return bridgeOK
}

// MARK: - URLs

@_cdecl("flequit_ios_open_url")
public func flequitIosOpenURL(_ url: UnsafePointer<CChar>?) -> Int32 {
    guard let text = string(from: url), let target = URL(string: text) else {
        return bridgeFailed
    }
    return onMainSync {
        guard UIApplication.shared.canOpenURL(target) else { return bridgeFailed }
        UIApplication.shared.open(target)
        return bridgeOK
    }
}

// MARK: - Document picker

typealias DocumentCallback = @convention(c) (
    UnsafeMutableRawPointer?, UnsafePointer<CChar>?, UnsafePointer<CChar>?
) -> Void

/// Presents a document picker and reports the result exactly once.
///
/// Held by the presenting controller through its delegate, and released when
/// the callback has fired — the picker guarantees exactly one of
/// `didPickDocumentsAt` / `wasCancelled`.
private final class DocumentPickerDelegate: NSObject, UIDocumentPickerDelegate {
    private let callback: DocumentCallback
    private let context: UnsafeMutableRawPointer?
    private var reported = false
    /// Keeps `self` alive while UIKit holds only a weak delegate reference.
    private var retained: DocumentPickerDelegate?

    init(callback: @escaping DocumentCallback, context: UnsafeMutableRawPointer?) {
        self.callback = callback
        self.context = context
        super.init()
        self.retained = self
    }

    func documentPicker(
        _ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]
    ) {
        guard let url = urls.first else {
            report(reference: nil, displayName: nil)
            return
        }
        // A security-scoped bookmark is what survives a restart; the plain URL
        // stops being readable as soon as the picker's grant lapses.
        let reference: String
        if let bookmark = try? url.bookmarkData(
            options: .minimalBookmark, includingResourceValuesForKeys: nil, relativeTo: nil)
        {
            reference = bookmark.base64EncodedString()
        } else {
            NSLog("flequit: could not bookmark the selected document; using the raw url")
            reference = url.absoluteString
        }
        report(reference: reference, displayName: url.lastPathComponent)
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        report(reference: nil, displayName: nil)
    }

    private func report(reference: String?, displayName: String?) {
        guard !reported else { return }
        reported = true
        withOptionalCString(reference) { referencePointer in
            withOptionalCString(displayName) { namePointer in
                callback(context, referencePointer, namePointer)
            }
        }
        retained = nil
    }

    private func withOptionalCString(
        _ value: String?, _ body: (UnsafePointer<CChar>?) -> Void
    ) {
        guard let value else {
            body(nil)
            return
        }
        value.withCString { body($0) }
    }
}

private func presentPicker(
    _ makeController: @escaping () -> UIDocumentPickerViewController,
    _ callback: @escaping DocumentCallback,
    _ context: UnsafeMutableRawPointer?
) -> Int32 {
    onMainSync {
        guard let presenter = topViewController() else {
            NSLog("flequit: no view controller to present the document picker from")
            return bridgeFailed
        }
        let controller = makeController()
        let delegate = DocumentPickerDelegate(callback: callback, context: context)
        controller.delegate = delegate
        presenter.present(controller, animated: true)
        return bridgeOK
    }
}

@_cdecl("flequit_ios_pick_document")
public func flequitIosPickDocument(
    _ contentType: UnsafePointer<CChar>?,
    _ callback: @escaping DocumentCallback,
    _ context: UnsafeMutableRawPointer?
) -> Int32 {
    let identifier = string(from: contentType) ?? "public.data"
    return presentPicker(
        {
            UIDocumentPickerViewController(
                forOpeningContentTypes: [UTType(identifier) ?? .data], asCopy: false)
        }, callback, context)
}

@_cdecl("flequit_ios_create_document")
public func flequitIosCreateDocument(
    _ suggestedName: UnsafePointer<CChar>?,
    _ callback: @escaping DocumentCallback,
    _ context: UnsafeMutableRawPointer?
) -> Int32 {
    let name = string(from: suggestedName) ?? "flequit.json"
    // The picker can only export a file that already exists, so an empty one is
    // staged in tmp/ and the user chooses where it lands.
    let staged = FileManager.default.temporaryDirectory.appendingPathComponent(name)
    if !FileManager.default.fileExists(atPath: staged.path) {
        FileManager.default.createFile(atPath: staged.path, contents: Data())
    }
    return presentPicker(
        { UIDocumentPickerViewController(forExporting: [staged], asCopy: false) },
        callback, context)
}

// MARK: - Document access

/// Resolves a reference produced by the picker back into a usable URL.
private func resolve(reference: String) -> (url: URL, scoped: Bool)? {
    if let data = Data(base64Encoded: reference) {
        var stale = false
        if let url = try? URL(
            resolvingBookmarkData: data, options: [], relativeTo: nil,
            bookmarkDataIsStale: &stale)
        {
            return (url, true)
        }
    }
    guard let url = URL(string: reference) else { return nil }
    return (url, false)
}

@_cdecl("flequit_ios_read_document")
public func flequitIosReadDocument(
    _ reference: UnsafePointer<CChar>?,
    _ outBytes: UnsafeMutablePointer<UnsafeMutablePointer<UInt8>?>?,
    _ outLength: UnsafeMutablePointer<Int>?
) -> Int32 {
    guard let text = string(from: reference), let resolved = resolve(reference: text),
        let outBytes, let outLength
    else { return bridgeFailed }

    if resolved.scoped {
        guard resolved.url.startAccessingSecurityScopedResource() else { return bridgeFailed }
    }
    defer {
        if resolved.scoped { resolved.url.stopAccessingSecurityScopedResource() }
    }

    guard let data = try? Data(contentsOf: resolved.url) else { return bridgeFailed }

    // Rust copies out of this buffer and then calls `flequit_ios_free`.
    let buffer = UnsafeMutablePointer<UInt8>.allocate(capacity: max(data.count, 1))
    data.copyBytes(to: buffer, count: data.count)
    outBytes.pointee = buffer
    outLength.pointee = data.count
    return bridgeOK
}

@_cdecl("flequit_ios_write_document")
public func flequitIosWriteDocument(
    _ reference: UnsafePointer<CChar>?,
    _ bytes: UnsafePointer<UInt8>?,
    _ length: Int
) -> Int32 {
    guard let text = string(from: reference), let resolved = resolve(reference: text), let bytes
    else { return bridgeFailed }

    if resolved.scoped {
        guard resolved.url.startAccessingSecurityScopedResource() else { return bridgeFailed }
    }
    defer {
        if resolved.scoped { resolved.url.stopAccessingSecurityScopedResource() }
    }

    let data = Data(bytes: bytes, count: length)
    do {
        try data.write(to: resolved.url, options: .atomic)
        return bridgeOK
    } catch {
        NSLog("flequit: could not write the selected document: \(error)")
        return bridgeFailed
    }
}

@_cdecl("flequit_ios_free")
public func flequitIosFree(_ bytes: UnsafeMutablePointer<UInt8>?, _ length: Int) {
    bytes?.deallocate()
}

// MARK: - Lifecycle

/// Forwards UIKit's lifecycle notifications to Rust.
///
/// Registered once by the app delegate; the observers live for the whole
/// process, so there is nothing to tear down.
enum LifecycleForwarder {
    static func install() {
        let center = NotificationCenter.default
        center.addObserver(
            forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: nil
        ) { _ in flequit_ios_lifecycle_event(lifecycleSuspend) }

        center.addObserver(
            forName: UIApplication.willEnterForegroundNotification, object: nil, queue: nil
        ) { _ in flequit_ios_lifecycle_event(lifecycleResume) }

        center.addObserver(
            forName: UIApplication.didReceiveMemoryWarningNotification, object: nil, queue: nil
        ) { _ in flequit_ios_lifecycle_event(lifecycleLowMemory) }
    }
}
