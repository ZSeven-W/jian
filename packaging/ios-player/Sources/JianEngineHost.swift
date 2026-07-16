import Foundation
import Metal
import QuartzCore
import UIKit

struct JianOwnedTextState {
    let windowText: String
    let windowStart: UInt32
    let selectionStart: UInt32
    let selectionEnd: UInt32
    let composingRange: Range<UInt32>?
}

struct JianFieldConfiguration {
    let inputKind: Int32
    let returnKeyHint: Int32
}

enum JianCallOrigin {
    case idle
    case frame
    case mutation
}

private final class JianDisplayLinkTarget: NSObject {
    weak var host: JianEngineHost?

    @objc func tick(_ link: CADisplayLink) {
        host?.displayLinkDidFire(link)
    }
}

final class JianEngineHost: NSObject {
    weak var view: JianPlayerView?

    private(set) var engine: OpaquePointer?
    private weak var surfaceLayer: CAMetalLayer?
    private var logicalSize = CGSize.zero
    private var scale: CGFloat = 1
    private var isSuspended = false
    private var isAlive = true
    private var callOrigin = JianCallOrigin.idle
    private var displayLink: CADisplayLink?
    private let displayLinkTarget = JianDisplayLinkTarget()
    private var wakeWork: DispatchWorkItem?
    private var observers: [NSObjectProtocol] = []

    var capabilityTasks: [UInt64: URLSessionTask] = [:]
    var capabilityAlerts: [UInt64: UIAlertController] = [:]
    var activeCapabilityIDs: Set<UInt64> = []

    var platformMarkedText: String?
    var fieldConfiguration = JianFieldConfiguration(
        inputKind: Int32(JianInputKind_Text.rawValue),
        returnKeyHint: Int32(JianReturnKeyHint_Default.rawValue)
    )

    override init() {
        super.init()
        displayLinkTarget.host = self
        let link = CADisplayLink(target: displayLinkTarget, selector: #selector(JianDisplayLinkTarget.tick(_:)))
        link.add(to: .main, forMode: .common)
        link.isPaused = true
        displayLink = link

        let center = NotificationCenter.default
        observers.append(center.addObserver(
            forName: UIApplication.didEnterBackgroundNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.suspendForBackground()
        })
        observers.append(center.addObserver(
            forName: UIApplication.willEnterForegroundNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.resumeFromBackground()
        })
    }

    deinit {
        displayLink?.invalidate()
        observers.forEach(NotificationCenter.default.removeObserver)
    }

    func configure(surface: CAMetalLayer, logicalSize size: CGSize, scale newScale: CGFloat) {
        precondition(Thread.isMainThread)
        guard isAlive, size.width > 0, size.height > 0, newScale > 0 else { return }
        surfaceLayer = surface
        logicalSize = size
        scale = newScale

        if engine == nil {
            createAndAttach(surface: surface)
        } else {
            resize(to: size, scale: newScale)
        }
    }

    func teardown() {
        precondition(Thread.isMainThread)
        guard isAlive else { return }
        wakeWork?.cancel()
        wakeWork = nil
        displayLink?.isPaused = true
        displayLink?.invalidate()
        displayLink = nil

        if let engine {
            _ = performCall(.mutation) { jian_suspend(engine) }
            isSuspended = true
            let status = performCall(.mutation) { jian_destroy(engine) }
            if status != JianStatus_Ok.rawValue {
                NSLog("Jian destroy failed with status %d", status)
            }
            self.engine = nil
        }
        isAlive = false
        cancelAllCapabilities()
        observers.forEach(NotificationCenter.default.removeObserver)
        observers.removeAll()
    }

    private func createAndAttach(surface: CAMetalLayer) {
        guard
            let documentURL = Bundle.main.url(forResource: "m1_acceptance", withExtension: "op"),
            let document = try? Data(contentsOf: documentURL)
        else {
            NSLog("Jian Player could not load bundled m1_acceptance.op")
            return
        }

        let storageURL: URL
        do {
            storageURL = try FileManager.default.url(
                for: .documentDirectory,
                in: .userDomainMask,
                appropriateFor: nil,
                create: true
            )
        } catch {
            NSLog("Jian Player could not resolve its storage directory: %@", error.localizedDescription)
            return
        }

        let storage = Data(storageURL.path.utf8)
        let assetBase = Data((Bundle.main.resourceURL?.path ?? "").utf8)
        var callbacks = makeCallbacks()
        var created: OpaquePointer?

        let status = document.withUnsafeBytes { documentBytes in
            storage.withUnsafeBytes { storageBytes in
                assetBase.withUnsafeBytes { assetBytes in
                    withUnsafePointer(to: &callbacks) { callbacksPointer in
                        var desc = JianCreateDesc()
                        desc.size = MemoryLayout<JianCreateDesc>.size
                        desc.doc_ptr = documentBytes.bindMemory(to: UInt8.self).baseAddress
                        desc.doc_len = documentBytes.count
                        desc.width = Float(logicalSize.width)
                        desc.height = Float(logicalSize.height)
                        desc.dpr = Float(scale)
                        desc.storage_dir_ptr = storageBytes.bindMemory(to: UInt8.self).baseAddress
                        desc.storage_dir_len = storageBytes.count
                        desc.callbacks = callbacksPointer
                        desc.asset_base_ptr = assetBytes.bindMemory(to: UInt8.self).baseAddress
                        desc.asset_base_len = assetBytes.count
                        return performCall(.mutation) { jian_create(&desc, &created) }
                    }
                }
            }
        }

        guard status == JianStatus_Ok.rawValue, let created else {
            reportFailure(status, operation: "jian_create", engine: nil)
            return
        }
        engine = created
        var surfaceDesc = JianSurfaceDesc()
        surfaceDesc.size = MemoryLayout<JianSurfaceDesc>.size
        surfaceDesc.handle = Unmanaged.passUnretained(surface).toOpaque()
        let attach = performCall(.mutation) { jian_attach_surface(created, &surfaceDesc) }
        guard attach == JianStatus_Ok.rawValue else {
            reportFailure(attach, operation: "jian_attach_surface", engine: created)
            _ = performCall(.mutation) { jian_destroy(created) }
            engine = nil
            return
        }
        isSuspended = false
    }

    private func makeCallbacks() -> JianCallbacks {
        var callbacks = JianCallbacks()
        callbacks.size = MemoryLayout<JianCallbacks>.size
        callbacks.user_data = Unmanaged.passUnretained(self).toOpaque()
        callbacks.needs_redraw = jianPlayerNeedsRedraw
        callbacks.runtime_error = jianPlayerRuntimeError
        callbacks.ime_control = jianPlayerImeControl
        callbacks.input_focus_changed = jianPlayerInputFocusChanged
        callbacks.text_state_changed = jianPlayerTextStateChanged
        callbacks.capability_request = jianPlayerCapabilityRequest
        callbacks.capability_cancelled = jianPlayerCapabilityCancelled
        return callbacks
    }

    private func resize(to size: CGSize, scale newScale: CGFloat) {
        guard let engine else { return }
        let status = performCall(.mutation) {
            jian_resize(engine, Float(size.width), Float(size.height), Float(newScale))
        }
        if status != JianStatus_Ok.rawValue {
            reportFailure(status, operation: "jian_resize", engine: engine)
        }
    }

    func updateSafeArea(_ insets: UIEdgeInsets) {
        precondition(Thread.isMainThread)
        guard let engine else { return }
        var top = max(0, min(insets.top, logicalSize.height))
        var bottom = max(0, min(insets.bottom, logicalSize.height))
        var left = max(0, min(insets.left, logicalSize.width))
        var right = max(0, min(insets.right, logicalSize.width))
        scalePair(&top, &bottom, extent: logicalSize.height)
        scalePair(&left, &right, extent: logicalSize.width)
        let status = performCall(.mutation) {
            jian_set_safe_area(engine, Float(top), Float(right), Float(bottom), Float(left))
        }
        if status != JianStatus_Ok.rawValue {
            reportFailure(status, operation: "jian_set_safe_area", engine: engine)
        }
    }

    func updateKeyboardHeight(_ height: CGFloat) {
        precondition(Thread.isMainThread)
        guard let engine else { return }
        let clamped = max(0, min(height, logicalSize.height))
        let status = performCall(.mutation) { jian_set_keyboard(engine, Float(clamped)) }
        if status != JianStatus_Ok.rawValue {
            reportFailure(status, operation: "jian_set_keyboard", engine: engine)
        }
    }

    func dispatchPointer(id: UInt32, phase: Int32, point: CGPoint) {
        precondition(Thread.isMainThread)
        guard let engine else { return }
        let status = performCall(.mutation) {
            jian_pointer(engine, id, phase, Float(point.x), Float(point.y), Self.nowMilliseconds())
        }
        if status != JianStatus_Ok.rawValue && status != JianStatus_Suspended.rawValue {
            reportFailure(status, operation: "jian_pointer", engine: engine)
        }
    }

    func displayLinkDidFire(_ link: CADisplayLink) {
        precondition(Thread.isMainThread)
        link.isPaused = true
        guard let engine, !isSuspended else { return }
        let status = performCall(.frame) { jian_frame(engine, Self.nowMilliseconds()) }
        if status == JianStatus_GpuError.rawValue {
            scheduleWake(at: Self.nowMilliseconds() + 17)
        } else if status != JianStatus_Ok.rawValue && status != JianStatus_Suspended.rawValue {
            reportFailure(status, operation: "jian_frame", engine: engine)
        }
    }

    private func suspendForBackground() {
        precondition(Thread.isMainThread)
        guard let engine, !isSuspended else { return }
        wakeWork?.cancel()
        displayLink?.isPaused = true
        let status = performCall(.mutation) { jian_suspend(engine) }
        if status == JianStatus_Ok.rawValue {
            isSuspended = true
        } else {
            reportFailure(status, operation: "jian_suspend", engine: engine)
        }
    }

    private func resumeFromBackground() {
        precondition(Thread.isMainThread)
        guard let engine, isSuspended, let surfaceLayer else { return }
        var desc = JianSurfaceDesc()
        desc.size = MemoryLayout<JianSurfaceDesc>.size
        desc.handle = Unmanaged.passUnretained(surfaceLayer).toOpaque()
        let status = performCall(.mutation) { jian_resume(engine, &desc) }
        if status == JianStatus_Ok.rawValue {
            isSuspended = false
        } else {
            reportFailure(status, operation: "jian_resume", engine: engine)
        }
    }

    func deferNeedsRedraw(hasNextWake: Bool, nextWakeMilliseconds: UInt64) {
        let originatedFromFrame = callOrigin == .frame
        DispatchQueue.main.async { [weak self] in
            guard let self, self.isAlive, !self.isSuspended else { return }
            if originatedFromFrame {
                if hasNextWake {
                    self.scheduleWake(at: nextWakeMilliseconds)
                } else {
                    self.wakeWork?.cancel()
                    self.wakeWork = nil
                    self.displayLink?.isPaused = true
                }
            } else {
                self.wakeWork?.cancel()
                self.wakeWork = nil
                self.displayLink?.isPaused = false
            }
        }
    }

    private func scheduleWake(at milliseconds: UInt64) {
        wakeWork?.cancel()
        let now = Self.nowMilliseconds()
        if milliseconds <= now {
            displayLink?.isPaused = false
            return
        }
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.isAlive, !self.isSuspended else { return }
            self.displayLink?.isPaused = false
        }
        wakeWork = work
        DispatchQueue.main.asyncAfter(
            deadline: .now() + Double(milliseconds - now) / 1_000,
            execute: work
        )
    }

    func deferRuntimeError(_ message: String, source: String, kind: Int32) {
        DispatchQueue.main.async {
            let suffix = source.isEmpty ? "" : " [\(source)]"
            NSLog("Jian runtime diagnostic kind=%d: %@%@", kind, message, suffix)
        }
    }

    func deferFocusChange(focused: Bool, configuration: JianFieldConfiguration?) {
        DispatchQueue.main.async { [weak self] in
            guard let self, self.isAlive, let view = self.view else { return }
            if let configuration {
                self.fieldConfiguration = configuration
                view.applyFieldConfiguration(configuration)
            }
            if focused {
                view.becomeFirstResponder()
                view.reloadInputViews()
            } else {
                view.resignFirstResponder()
                self.platformMarkedText = nil
            }
        }
    }

    func deferTextStateChanged() {
        DispatchQueue.main.async { [weak self] in
            guard let view = self?.view else { return }
            view.inputDelegate?.selectionWillChange(view)
            view.inputDelegate?.textWillChange(view)
            view.inputDelegate?.textDidChange(view)
            view.inputDelegate?.selectionDidChange(view)
        }
    }

    func deferImeControl(operation: Int32, requestID: UInt64) {
        DispatchQueue.main.async { [weak self] in
            guard let self, self.isAlive else { return }
            self.answerImeControl(operation: operation, requestID: requestID)
        }
    }

    private func answerImeControl(operation: Int32, requestID: UInt64) {
        guard let engine else { return }
        if operation == Int32(JianImeControlOp_Cancel.rawValue) {
            let status = performCall(.mutation) { jian_ime_cancel(engine, requestID) }
            if status != JianStatus_Ok.rawValue {
                reportFailure(status, operation: "jian_ime_cancel", engine: engine)
            } else {
                requestImmediateFrame()
            }
            platformMarkedText = nil
            return
        }

        let marked = platformMarkedText ?? composingTextFromEngine() ?? ""
        let bytes = Array(marked.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            performCall(.mutation) {
                jian_ime_commit(engine, buffer.baseAddress, buffer.count, 1, requestID)
            }
        }
        if status != JianStatus_Ok.rawValue {
            reportFailure(status, operation: "jian_ime_commit", engine: engine)
        } else {
            requestImmediateFrame()
        }
        platformMarkedText = nil
        if operation == Int32(JianImeControlOp_Dismiss.rawValue) {
            view?.resignFirstResponder()
        }
    }

    private func composingTextFromEngine() -> String? {
        guard let state = currentTextState(), let range = state.composingRange else { return nil }
        return text(in: range.lowerBound, end: range.upperBound)
    }

    @discardableResult
    func performCall(_ origin: JianCallOrigin = .mutation, _ body: () -> Int32) -> Int32 {
        precondition(Thread.isMainThread)
        let previous = callOrigin
        callOrigin = origin
        defer { callOrigin = previous }
        return body()
    }

    func requestImmediateFrame() {
        precondition(Thread.isMainThread)
        guard isAlive, !isSuspended else { return }
        wakeWork?.cancel()
        wakeWork = nil
        displayLink?.isPaused = false
    }

    func reportFailure(_ status: Int32, operation: String, engine: OpaquePointer?) {
        let detail = lastError(engine: engine)
        if detail.isEmpty {
            NSLog("%@ failed with JianStatus %d", operation, status)
        } else {
            NSLog("%@ failed with JianStatus %d: %@", operation, status, detail)
        }
    }

    private func lastError(engine: OpaquePointer?) -> String {
        var required = 0
        guard jian_last_error(engine, nil, 0, &required) == JianStatus_Ok.rawValue, required > 0 else {
            return ""
        }
        var bytes = [UInt8](repeating: 0, count: required)
        let status = bytes.withUnsafeMutableBufferPointer { buffer in
            jian_last_error(engine, buffer.baseAddress, buffer.count, &required)
        }
        guard status == JianStatus_Ok.rawValue else { return "" }
        return String(decoding: bytes.prefix(required), as: UTF8.self)
    }

    static func nowMilliseconds() -> UInt64 {
        UInt64((CACurrentMediaTime() * 1_000).rounded(.down))
    }
}

private func scalePair(_ first: inout CGFloat, _ second: inout CGFloat, extent: CGFloat) {
    let sum = first + second
    guard sum > extent, sum > 0 else { return }
    let factor = extent / sum
    first *= factor
    second *= factor
}

private func host(from userData: UnsafeMutableRawPointer?) -> JianEngineHost? {
    guard let userData else { return nil }
    return Unmanaged<JianEngineHost>.fromOpaque(userData).takeUnretainedValue()
}

private func copiedString(_ pointer: UnsafePointer<UInt8>?, _ length: Int) -> String {
    guard let pointer, length > 0 else { return "" }
    return String(decoding: UnsafeBufferPointer(start: pointer, count: length), as: UTF8.self)
}

private func jianPlayerNeedsRedraw(
    _ userData: UnsafeMutableRawPointer?,
    _ hasNextWake: Bool,
    _ nextWakeMilliseconds: UInt64
) {
    host(from: userData)?.deferNeedsRedraw(
        hasNextWake: hasNextWake,
        nextWakeMilliseconds: nextWakeMilliseconds
    )
}

private func jianPlayerRuntimeError(
    _ userData: UnsafeMutableRawPointer?,
    _ error: UnsafePointer<JianRuntimeError>?
) {
    guard let value = error?.pointee else { return }
    let message = copiedString(value.message_ptr, value.message_len)
    let source = copiedString(value.source_ptr, value.source_len)
    host(from: userData)?.deferRuntimeError(message, source: source, kind: value.kind)
}

private func jianPlayerImeControl(
    _ userData: UnsafeMutableRawPointer?,
    _ operation: Int32,
    _ requestID: UInt64
) {
    host(from: userData)?.deferImeControl(operation: operation, requestID: requestID)
}

private func jianPlayerInputFocusChanged(
    _ userData: UnsafeMutableRawPointer?,
    _ focused: Bool,
    _ info: UnsafePointer<JianFieldInfo>?
) {
    let configuration = info.map {
        JianFieldConfiguration(
            inputKind: $0.pointee.input_kind,
            returnKeyHint: $0.pointee.return_key_hint
        )
    }
    host(from: userData)?.deferFocusChange(focused: focused, configuration: configuration)
}

private func jianPlayerTextStateChanged(_ userData: UnsafeMutableRawPointer?) {
    host(from: userData)?.deferTextStateChanged()
}

private func jianPlayerCapabilityRequest(
    _ userData: UnsafeMutableRawPointer?,
    _ requestID: UInt64,
    _ request: UnsafePointer<JianCapabilityRequest>?
) {
    guard let request, let owned = JianOwnedCapabilityRequest(copying: request.pointee) else { return }
    host(from: userData)?.deferCapabilityRequest(id: requestID, request: owned)
}

private func jianPlayerCapabilityCancelled(
    _ userData: UnsafeMutableRawPointer?,
    _ requestID: UInt64
) {
    host(from: userData)?.deferCapabilityCancellation(id: requestID)
}
