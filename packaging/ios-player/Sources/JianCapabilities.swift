import Foundation
import UIKit

enum JianOwnedCapabilityRequest {
    case http(
        method: String,
        url: String,
        headers: [(String, String)],
        body: Data?,
        timeoutMilliseconds: UInt64?
    )
    case confirm(title: String, message: String)
    case clipboardRead
    case clipboardWrite(String)
    case imageFetch(String)
    case openURL(String)

    init?(copying request: JianCapabilityRequest) {
        switch request.kind {
        case Int32(JianCapabilityKind_HttpFetch.rawValue):
            let value = request.data.http_fetch
            var headers: [(String, String)] = []
            if let pointer = value.headers, value.headers_len > 0 {
                let buffer = UnsafeBufferPointer(start: pointer, count: value.headers_len)
                headers.reserveCapacity(buffer.count)
                for header in buffer {
                    headers.append((
                        Self.string(header.name_ptr, header.name_len),
                        Self.string(header.value_ptr, header.value_len)
                    ))
                }
            }
            let body = value.has_body ? Self.data(value.body_ptr, value.body_len) : nil
            self = .http(
                method: Self.string(value.method_ptr, value.method_len),
                url: Self.string(value.url_ptr, value.url_len),
                headers: headers,
                body: body,
                timeoutMilliseconds: value.has_timeout ? value.timeout_ms : nil
            )
        case Int32(JianCapabilityKind_Confirm.rawValue):
            let value = request.data.confirm
            self = .confirm(
                title: Self.string(value.title_ptr, value.title_len),
                message: Self.string(value.message_ptr, value.message_len)
            )
        case Int32(JianCapabilityKind_ClipboardRead.rawValue):
            self = .clipboardRead
        case Int32(JianCapabilityKind_ClipboardWrite.rawValue):
            let value = request.data.clipboard_write
            self = .clipboardWrite(Self.string(value.text_ptr, value.text_len))
        case Int32(JianCapabilityKind_ImageFetch.rawValue):
            let value = request.data.image_fetch
            self = .imageFetch(Self.string(value.url_ptr, value.url_len))
        case Int32(JianCapabilityKind_OpenUrl.rawValue):
            let value = request.data.open_url
            self = .openURL(Self.string(value.url_ptr, value.url_len))
        default:
            return nil
        }
    }

    private static func string(_ pointer: UnsafePointer<UInt8>?, _ length: Int) -> String {
        guard let pointer, length > 0 else { return "" }
        return String(decoding: UnsafeBufferPointer(start: pointer, count: length), as: UTF8.self)
    }

    private static func data(_ pointer: UnsafePointer<UInt8>?, _ length: Int) -> Data {
        guard let pointer, length > 0 else { return Data() }
        return Data(bytes: pointer, count: length)
    }
}

extension JianEngineHost {
    func deferCapabilityRequest(id: UInt64, request: JianOwnedCapabilityRequest) {
        DispatchQueue.main.async { [weak self] in
            guard let self, self.engine != nil else { return }
            self.activeCapabilityIDs.insert(id)
            self.performCapabilityRequest(id: id, request: request)
        }
    }

    func deferCapabilityCancellation(id: UInt64) {
        DispatchQueue.main.async { [weak self] in
            self?.cancelCapability(id: id)
        }
    }

    func cancelAllCapabilities() {
        capabilityTasks.values.forEach { $0.cancel() }
        capabilityTasks.removeAll()
        capabilityAlerts.values.forEach { $0.dismiss(animated: false) }
        capabilityAlerts.removeAll()
        activeCapabilityIDs.removeAll()
    }

    private func cancelCapability(id: UInt64) {
        capabilityTasks.removeValue(forKey: id)?.cancel()
        capabilityAlerts.removeValue(forKey: id)?.dismiss(animated: true)
        activeCapabilityIDs.remove(id)
    }

    private func performCapabilityRequest(id: UInt64, request: JianOwnedCapabilityRequest) {
        switch request {
        case let .http(method, url, headers, body, timeoutMilliseconds):
            performHTTP(
                id: id,
                method: method,
                url: url,
                headers: headers,
                body: body,
                timeoutMilliseconds: timeoutMilliseconds
            )
        case let .confirm(title, message):
            performConfirm(id: id, title: title, message: message)
        case .clipboardRead:
            // Reserved in v1, but implemented so the ABI branch is complete.
            let text = UIPasteboard.general.string ?? ""
            guard finishCapability(id: id) else { return }
            sendClipboardRead(id: id, text: text, error: nil)
        case let .clipboardWrite(text):
            // Reserved in v1, but implemented so a future shell trigger is safe.
            UIPasteboard.general.string = text
            guard finishCapability(id: id) else { return }
            sendClipboardWrite(id: id, error: nil)
        case let .imageFetch(url):
            performImageFetch(id: id, url: url)
        case let .openURL(value):
            performOpenURL(id: id, value: value)
        }
    }

    private func performHTTP(
        id: UInt64,
        method: String,
        url: String,
        headers: [(String, String)],
        body: Data?,
        timeoutMilliseconds: UInt64?
    ) {
        guard let url = URL(string: url) else {
            guard finishCapability(id: id) else { return }
            sendHTTP(id: id, response: nil, body: nil, error: "invalid URL")
            return
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        for (name, value) in headers {
            request.setValue(value, forHTTPHeaderField: name)
        }
        request.httpBody = body
        if let timeoutMilliseconds {
            request.timeoutInterval = max(0.001, Double(timeoutMilliseconds) / 1_000)
        }

        let task = URLSession.shared.dataTask(with: request) { [weak self] data, response, error in
            DispatchQueue.main.async {
                guard let self, self.finishCapability(id: id) else { return }
                if let error {
                    self.sendHTTP(id: id, response: nil, body: nil, error: error.localizedDescription)
                    return
                }
                guard let response = response as? HTTPURLResponse else {
                    self.sendHTTP(id: id, response: nil, body: nil, error: "non-HTTP response")
                    return
                }
                let body = data ?? Data()
                if body.count > Self.capabilityByteLimit {
                    self.sendHTTP(id: id, response: nil, body: nil, error: "response exceeds 64 MiB")
                } else {
                    self.sendHTTP(id: id, response: response, body: body, error: nil)
                }
            }
        }
        capabilityTasks[id] = task
        task.resume()
    }

    private func performImageFetch(id: UInt64, url: String) {
        guard let url = URL(string: url) else {
            guard finishCapability(id: id) else { return }
            sendImage(id: id, bytes: nil, error: "invalid URL")
            return
        }
        let task = URLSession.shared.dataTask(with: url) { [weak self] data, _, error in
            DispatchQueue.main.async {
                guard let self, self.finishCapability(id: id) else { return }
                if let error {
                    self.sendImage(id: id, bytes: nil, error: error.localizedDescription)
                    return
                }
                let bytes = data ?? Data()
                if bytes.count > Self.capabilityByteLimit {
                    self.sendImage(id: id, bytes: nil, error: "image exceeds 64 MiB")
                } else {
                    self.sendImage(id: id, bytes: bytes, error: nil)
                }
            }
        }
        capabilityTasks[id] = task
        task.resume()
    }

    private func performConfirm(id: UInt64, title: String, message: String) {
        guard let presenter = view?.nearestViewController else {
            guard finishCapability(id: id) else { return }
            sendConfirm(id: id, value: false)
            return
        }
        let alert = UIAlertController(title: title, message: message, preferredStyle: .alert)
        alert.addAction(UIAlertAction(
            title: NSLocalizedString("Cancel", comment: "Default confirmation cancel button"),
            style: .cancel
        ) { [weak self] _ in
            guard let self, self.finishCapability(id: id) else { return }
            self.sendConfirm(id: id, value: false)
        })
        alert.addAction(UIAlertAction(
            title: NSLocalizedString("OK", comment: "Default confirmation accept button"),
            style: .default
        ) { [weak self] _ in
            guard let self, self.finishCapability(id: id) else { return }
            self.sendConfirm(id: id, value: true)
        })
        capabilityAlerts[id] = alert
        presenter.present(alert, animated: true)
    }

    private func performOpenURL(id: UInt64, value: String) {
        guard let url = URL(string: value) else {
            guard finishCapability(id: id) else { return }
            sendOpenURL(id: id, succeeded: false, error: "invalid URL")
            return
        }
        UIApplication.shared.open(url, options: [:]) { [weak self] succeeded in
            DispatchQueue.main.async {
                guard let self, self.finishCapability(id: id) else { return }
                self.sendOpenURL(
                    id: id,
                    succeeded: succeeded,
                    error: succeeded ? nil : "UIApplication refused the URL"
                )
            }
        }
    }

    private func finishCapability(id: UInt64) -> Bool {
        guard activeCapabilityIDs.remove(id) != nil else { return false }
        capabilityTasks.removeValue(forKey: id)
        capabilityAlerts.removeValue(forKey: id)
        return true
    }

    private func sendHTTP(id: UInt64, response: HTTPURLResponse?, body: Data?, error: String?) {
        let headerPairs: [(String, String)] = response?.allHeaderFields.compactMap { key, value in
            guard let key = key as? String else { return nil }
            return (key, String(describing: value))
        } ?? []
        let names = headerPairs.map { Data($0.0.utf8) as NSData }
        let values = headerPairs.map { Data($0.1.utf8) as NSData }
        let headers: [JianHeader] = headerPairs.indices.map { index in
            var header = JianHeader()
            header.name_ptr = Self.pointer(names[index])
            header.name_len = names[index].length
            header.value_ptr = Self.pointer(values[index])
            header.value_len = values[index].length
            return header
        }
        let bodyData = (body ?? Data()) as NSData
        let errorData = Data((error ?? "").utf8) as NSData
        headers.withUnsafeBufferPointer { headerBuffer in
            var value = JianHttpFetchResult()
            value.ok = error == nil && response != nil
            value.status = UInt16(clamping: response?.statusCode ?? 0)
            value.headers = headerBuffer.baseAddress
            value.headers_len = headerBuffer.count
            value.body_ptr = Self.pointer(bodyData)
            value.body_len = bodyData.length
            value.error_ptr = Self.pointer(errorData)
            value.error_len = errorData.length
            var data = JianCapabilityResultData()
            data.http_fetch = value
            sendCapabilityResult(id: id, kind: Int32(JianCapabilityKind_HttpFetch.rawValue), data: data)
        }
    }

    private func sendConfirm(id: UInt64, value: Bool) {
        var result = JianConfirmResult()
        result.value = value
        var data = JianCapabilityResultData()
        data.confirm = result
        sendCapabilityResult(id: id, kind: Int32(JianCapabilityKind_Confirm.rawValue), data: data)
    }

    private func sendClipboardRead(id: UInt64, text: String?, error: String?) {
        let textData = Data((text ?? "").utf8) as NSData
        let errorData = Data((error ?? "").utf8) as NSData
        var value = JianClipboardReadResult()
        value.ok = error == nil
        value.text_ptr = Self.pointer(textData)
        value.text_len = textData.length
        value.error_ptr = Self.pointer(errorData)
        value.error_len = errorData.length
        var data = JianCapabilityResultData()
        data.clipboard_read = value
        sendCapabilityResult(id: id, kind: Int32(JianCapabilityKind_ClipboardRead.rawValue), data: data)
    }

    private func sendClipboardWrite(id: UInt64, error: String?) {
        let errorData = Data((error ?? "").utf8) as NSData
        var value = JianClipboardWriteResult()
        value.ok = error == nil
        value.error_ptr = Self.pointer(errorData)
        value.error_len = errorData.length
        var data = JianCapabilityResultData()
        data.clipboard_write = value
        sendCapabilityResult(id: id, kind: Int32(JianCapabilityKind_ClipboardWrite.rawValue), data: data)
    }

    private func sendImage(id: UInt64, bytes: Data?, error: String?) {
        let imageData = (bytes ?? Data()) as NSData
        let errorData = Data((error ?? "").utf8) as NSData
        var value = JianImageFetchResult()
        value.ok = error == nil && bytes != nil
        value.bytes_ptr = Self.pointer(imageData)
        value.bytes_len = imageData.length
        value.error_ptr = Self.pointer(errorData)
        value.error_len = errorData.length
        var data = JianCapabilityResultData()
        data.image_fetch = value
        sendCapabilityResult(id: id, kind: Int32(JianCapabilityKind_ImageFetch.rawValue), data: data)
    }

    private func sendOpenURL(id: UInt64, succeeded: Bool, error: String?) {
        let errorData = Data((error ?? "").utf8) as NSData
        var value = JianOpenUrlResult()
        value.ok = succeeded
        value.error_ptr = Self.pointer(errorData)
        value.error_len = errorData.length
        var data = JianCapabilityResultData()
        data.open_url = value
        sendCapabilityResult(id: id, kind: Int32(JianCapabilityKind_OpenUrl.rawValue), data: data)
    }

    private func sendCapabilityResult(id: UInt64, kind: Int32, data: JianCapabilityResultData) {
        guard let engine else { return }
        var result = JianCapabilityResult()
        result.size = MemoryLayout<JianCapabilityResult>.size
        result.kind = kind
        result.data = data
        let status = performCall { jian_capability_result(engine, id, &result) }
        if status != JianStatus_Ok.rawValue {
            reportFailure(status, operation: "jian_capability_result", engine: engine)
        } else {
            requestImmediateFrame()
        }
    }

    private static func pointer(_ data: NSData) -> UnsafePointer<UInt8>? {
        guard data.length > 0 else { return nil }
        return data.bytes.assumingMemoryBound(to: UInt8.self)
    }

    private static let capabilityByteLimit = 64 * 1_024 * 1_024
}

private extension UIView {
    var nearestViewController: UIViewController? {
        var responder: UIResponder? = self
        while let current = responder {
            if let controller = current as? UIViewController { return controller }
            responder = current.next
        }
        return window?.rootViewController
    }
}
