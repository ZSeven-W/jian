import Foundation
import UIKit

final class JianTextPosition: UITextPosition {
    let offset: UInt32

    init(_ offset: UInt32) {
        self.offset = offset
        super.init()
    }
}

final class JianTextRange: UITextRange {
    let lower: UInt32
    let upper: UInt32

    init(_ first: UInt32, _ second: UInt32) {
        lower = min(first, second)
        upper = max(first, second)
        super.init()
    }

    override var start: UITextPosition { JianTextPosition(lower) }
    override var end: UITextPosition { JianTextPosition(upper) }
    override var isEmpty: Bool { lower == upper }
}

final class JianSelectionRect: UITextSelectionRect {
    private let value: CGRect
    private let direction: NSWritingDirection
    private let starts: Bool
    private let ends: Bool

    init(rect: CGRect, direction: NSWritingDirection, containsStart: Bool, containsEnd: Bool) {
        value = rect
        self.direction = direction
        starts = containsStart
        ends = containsEnd
        super.init()
    }

    override var rect: CGRect { value }
    override var writingDirection: NSWritingDirection { direction }
    override var containsStart: Bool { starts }
    override var containsEnd: Bool { ends }
    override var isVertical: Bool { false }
}

extension JianEngineHost {
    func currentTextState() -> JianOwnedTextState? {
        guard let engine else { return nil }
        var state = JianTextState()
        state.size = MemoryLayout<JianTextState>.size
        let status = performCall { jian_text_get_state(engine, &state) }
        guard status == JianStatus_Ok.rawValue else { return nil }
        let text: String
        if let pointer = state.text_ptr, state.text_len > 0 {
            text = String(
                decoding: UnsafeBufferPointer(start: pointer, count: state.text_len),
                as: UTF8.self
            )
        } else {
            text = ""
        }
        let composing = state.has_composing
            ? min(state.composing_start, state.composing_end)..<max(state.composing_start, state.composing_end)
            : nil
        return JianOwnedTextState(
            windowText: text,
            windowStart: state.window_start,
            selectionStart: state.selection_start,
            selectionEnd: state.selection_end,
            composingRange: composing
        )
    }

    func text(in start: UInt32, end: UInt32) -> String? {
        guard let engine else { return nil }
        var required = 0
        var status = performCall {
            jian_text_get_range(engine, start, end, nil, 0, &required)
        }
        guard status == JianStatus_Ok.rawValue else { return nil }
        if required == 0 { return "" }
        var bytes = [UInt8](repeating: 0, count: required)
        status = bytes.withUnsafeMutableBufferPointer { buffer in
            performCall {
                jian_text_get_range(engine, start, end, buffer.baseAddress, buffer.count, &required)
            }
        }
        guard status == JianStatus_Ok.rawValue else { return nil }
        return String(decoding: bytes.prefix(required), as: UTF8.self)
    }

    func documentText() -> String? {
        text(in: 0, end: UInt32.max)
    }

    func insertText(_ value: String) -> Bool {
        guard let engine else { return false }
        let bytes = Array(value.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            performCall { jian_text_insert(engine, buffer.baseAddress, buffer.count) }
        }
        return handleTextStatus(status, operation: "jian_text_insert")
    }

    func replaceText(in range: JianTextRange, with value: String) -> Bool {
        guard let engine else { return false }
        let bytes = Array(value.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            performCall {
                jian_text_replace_range(
                    engine,
                    range.lower,
                    range.upper,
                    buffer.baseAddress,
                    buffer.count
                )
            }
        }
        return handleTextStatus(status, operation: "jian_text_replace_range")
    }

    func setSelection(_ range: JianTextRange) -> Bool {
        guard let engine else { return false }
        let status = performCall { jian_text_set_selection(engine, range.lower, range.upper) }
        return handleTextStatus(status, operation: "jian_text_set_selection")
    }

    func setComposingText(_ value: String, selection: NSRange) -> Bool {
        guard let engine else { return false }
        let utf16Length = UInt32(clamping: value.utf16.count)
        let start = clampedUInt32(selection.location, maximum: utf16Length)
        let end: UInt32
        if selection.location == NSNotFound {
            end = start
        } else {
            let sum = selection.location.addingReportingOverflow(selection.length)
            end = clampedUInt32(sum.overflow ? Int.max : sum.partialValue, maximum: utf16Length)
        }
        let bytes = Array(value.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            performCall {
                jian_ime_set_composing_text(
                    engine,
                    buffer.baseAddress,
                    buffer.count,
                    start,
                    end
                )
            }
        }
        let succeeded = handleTextStatus(status, operation: "jian_ime_set_composing_text")
        if succeeded { platformMarkedText = value }
        return succeeded
    }

    func commitPlatformComposition() -> Bool {
        commitText(platformMarkedText ?? currentComposingText() ?? "")
    }

    func commitText(_ text: String) -> Bool {
        guard let engine else { return false }
        let bytes = Array(text.utf8)
        let status = bytes.withUnsafeBufferPointer { buffer in
            performCall { jian_ime_commit(engine, buffer.baseAddress, buffer.count, 1, 0) }
        }
        let succeeded = handleTextStatus(status, operation: "jian_ime_commit")
        if succeeded { platformMarkedText = nil }
        return succeeded
    }

    func cancelPlatformComposition() -> Bool {
        guard let engine else { return false }
        let status = performCall { jian_ime_cancel(engine, 0) }
        let succeeded = handleTextStatus(status, operation: "jian_ime_cancel")
        if succeeded { platformMarkedText = nil }
        return succeeded
    }

    func withTextBatch(_ body: () -> Void) {
        guard let engine else { return }
        let begin = performCall { jian_text_batch_begin(engine) }
        guard begin == JianStatus_Ok.rawValue else {
            if begin != JianStatus_NoFocus.rawValue {
                reportFailure(begin, operation: "jian_text_batch_begin", engine: engine)
            }
            return
        }
        body()
        let end = performCall { jian_text_batch_end(engine) }
        if end != JianStatus_Ok.rawValue {
            reportFailure(end, operation: "jian_text_batch_end", engine: engine)
        }
    }

    func currentCaretRect() -> CGRect? {
        guard let engine else { return nil }
        var output = JianRect()
        let status = performCall { jian_text_caret_rect(engine, &output) }
        return status == JianStatus_Ok.rawValue ? output.cgRect : nil
    }

    func caretRect(at offset: UInt32) -> CGRect? {
        guard let engine else { return nil }
        var output = JianRect()
        let status = performCall { jian_text_caret_rect_for_offset(engine, offset, &output) }
        return status == JianStatus_Ok.rawValue ? output.cgRect : nil
    }

    func textRects(in range: JianTextRange) -> [JianTextRect]? {
        guard let engine else { return nil }
        var count = 0
        var status = performCall {
            jian_text_rects_for_range(engine, range.lower, range.upper, nil, 0, &count)
        }
        guard status == JianStatus_Ok.rawValue else { return nil }
        if count == 0 { return [] }
        var output = [JianTextRect](repeating: JianTextRect(), count: count)
        status = output.withUnsafeMutableBufferPointer { buffer in
            performCall {
                jian_text_rects_for_range(
                    engine,
                    range.lower,
                    range.upper,
                    buffer.baseAddress,
                    buffer.count,
                    &count
                )
            }
        }
        guard status == JianStatus_Ok.rawValue else { return nil }
        return Array(output.prefix(min(output.count, count)))
    }

    func textPosition(at point: CGPoint) -> UInt32? {
        guard let engine else { return nil }
        var output: UInt32 = 0
        let status = performCall {
            jian_text_position_at_point(engine, Float(point.x), Float(point.y), &output)
        }
        return status == JianStatus_Ok.rawValue ? output : nil
    }

    func textRange(at point: CGPoint, granularity: Int32) -> JianTextRange? {
        guard let engine else { return nil }
        var start: UInt32 = 0
        var end: UInt32 = 0
        let status = performCall {
            jian_text_range_at_point(
                engine,
                Float(point.x),
                Float(point.y),
                granularity,
                &start,
                &end
            )
        }
        return status == JianStatus_Ok.rawValue ? JianTextRange(start, end) : nil
    }

    private func currentComposingText() -> String? {
        guard let range = currentTextState()?.composingRange else { return nil }
        return text(in: range.lowerBound, end: range.upperBound)
    }

    private func handleTextStatus(_ status: Int32, operation: String) -> Bool {
        if status == JianStatus_Ok.rawValue {
            requestImmediateFrame()
            return true
        }
        if status != JianStatus_NoFocus.rawValue, let engine {
            reportFailure(status, operation: operation, engine: engine)
        }
        return false
    }
}

extension JianPlayerView: UITextInput {
    var hasText: Bool {
        guard let text = host.documentText() else { return false }
        return !text.isEmpty
    }

    func insertText(_ text: String) {
        if markedTextRange != nil {
            _ = host.commitText(text)
        } else {
            host.withTextBatch { _ = host.insertText(text) }
        }
    }

    func deleteBackward() {
        guard let selection = selectedTextRange as? JianTextRange else { return }
        if !selection.isEmpty {
            host.withTextBatch { _ = host.replaceText(in: selection, with: "") }
            return
        }
        guard selection.lower > 0, let text = host.documentText() else { return }
        let string = text as NSString
        let index = min(Int(selection.lower) - 1, max(0, string.length - 1))
        guard string.length > 0 else { return }
        let deletion = string.rangeOfComposedCharacterSequence(at: index)
        let range = JianTextRange(UInt32(deletion.location), UInt32(deletion.location + deletion.length))
        host.withTextBatch { _ = host.replaceText(in: range, with: "") }
    }

    func text(in range: UITextRange) -> String? {
        guard let range = range as? JianTextRange else { return nil }
        return host.text(in: range.lower, end: range.upper)
    }

    func replace(_ range: UITextRange, withText text: String) {
        guard let range = range as? JianTextRange else { return }
        host.withTextBatch { _ = host.replaceText(in: range, with: text) }
    }

    var selectedTextRange: UITextRange? {
        get {
            guard let state = host.currentTextState() else { return nil }
            return JianTextRange(state.selectionStart, state.selectionEnd)
        }
        set {
            guard let range = newValue as? JianTextRange else { return }
            _ = host.setSelection(range)
        }
    }

    var markedTextRange: UITextRange? {
        guard let range = host.currentTextState()?.composingRange else { return nil }
        return JianTextRange(range.lowerBound, range.upperBound)
    }

    func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        guard let markedText else {
            unmarkText()
            return
        }
        _ = host.setComposingText(markedText, selection: selectedRange)
    }

    func unmarkText() {
        _ = host.commitPlatformComposition()
    }

    var beginningOfDocument: UITextPosition { JianTextPosition(0) }

    var endOfDocument: UITextPosition {
        JianTextPosition(documentLength)
    }

    func textRange(from fromPosition: UITextPosition, to toPosition: UITextPosition) -> UITextRange? {
        guard let first = positionOffset(fromPosition), let second = positionOffset(toPosition) else { return nil }
        return JianTextRange(first, second)
    }

    func position(from position: UITextPosition, offset: Int) -> UITextPosition? {
        guard let value = positionOffset(position) else { return nil }
        let candidate = Int64(value) + Int64(offset)
        return JianTextPosition(UInt32(clamping: max(0, min(Int64(documentLength), candidate))))
    }

    func position(from position: UITextPosition, in direction: UITextLayoutDirection, offset: Int) -> UITextPosition? {
        let signed: Int
        switch direction {
        case .left, .up: signed = -offset
        case .right, .down: signed = offset
        @unknown default: signed = offset
        }
        return self.position(from: position, offset: signed)
    }

    func compare(_ position: UITextPosition, to other: UITextPosition) -> ComparisonResult {
        guard let lhs = positionOffset(position), let rhs = positionOffset(other) else { return .orderedSame }
        if lhs < rhs { return .orderedAscending }
        if lhs > rhs { return .orderedDescending }
        return .orderedSame
    }

    func offset(from: UITextPosition, to toPosition: UITextPosition) -> Int {
        guard let lhs = positionOffset(from), let rhs = positionOffset(toPosition) else { return 0 }
        return Int(Int64(rhs) - Int64(lhs))
    }

    func position(within range: UITextRange, farthestIn direction: UITextLayoutDirection) -> UITextPosition? {
        guard let range = range as? JianTextRange else { return nil }
        switch direction {
        case .left, .up: return JianTextPosition(range.lower)
        case .right, .down: return JianTextPosition(range.upper)
        @unknown default: return JianTextPosition(range.upper)
        }
    }

    func characterRange(byExtending position: UITextPosition, in direction: UITextLayoutDirection) -> UITextRange? {
        guard let offset = positionOffset(position), let text = host.documentText() else { return nil }
        let string = text as NSString
        guard string.length > 0 else { return JianTextRange(0, 0) }
        let index: Int
        switch direction {
        case .left, .up: index = max(0, min(Int(offset) - 1, string.length - 1))
        case .right, .down: index = max(0, min(Int(offset), string.length - 1))
        @unknown default: index = max(0, min(Int(offset), string.length - 1))
        }
        let composed = string.rangeOfComposedCharacterSequence(at: index)
        return JianTextRange(UInt32(composed.location), UInt32(composed.location + composed.length))
    }

    func baseWritingDirection(for position: UITextPosition, in direction: UITextStorageDirection) -> NSWritingDirection {
        guard let offset = positionOffset(position) else { return .natural }
        let end = min(documentLength, offset &+ (offset == UInt32.max ? 0 : 1))
        guard let rect = host.textRects(in: JianTextRange(offset, end))?.first else { return .natural }
        return rect.writing_direction == Int32(JianWritingDirection_RightToLeft.rawValue)
            ? .rightToLeft
            : .leftToRight
    }

    func setBaseWritingDirection(_ writingDirection: NSWritingDirection, for range: UITextRange) {
        // The document owns paragraph direction; the v1 ABI intentionally has no setter.
    }

    func firstRect(for range: UITextRange) -> CGRect {
        guard let range = range as? JianTextRange else { return .zero }
        if range.isEmpty {
            if let state = host.currentTextState(), state.selectionEnd == range.upper {
                return host.currentCaretRect() ?? .zero
            }
            return host.caretRect(at: range.upper) ?? .zero
        }
        return host.textRects(in: range)?.first?.rect.cgRect ?? .zero
    }

    func caretRect(for position: UITextPosition) -> CGRect {
        guard let offset = positionOffset(position) else { return .zero }
        return host.caretRect(at: offset) ?? .zero
    }

    func selectionRects(for range: UITextRange) -> [UITextSelectionRect] {
        guard let range = range as? JianTextRange, let values = host.textRects(in: range) else { return [] }
        return values.enumerated().map { index, value in
            JianSelectionRect(
                rect: value.rect.cgRect,
                direction: value.writing_direction == Int32(JianWritingDirection_RightToLeft.rawValue)
                    ? .rightToLeft
                    : .leftToRight,
                containsStart: index == values.startIndex,
                containsEnd: index == values.index(before: values.endIndex)
            )
        }
    }

    func closestPosition(to point: CGPoint) -> UITextPosition? {
        host.textPosition(at: point).map(JianTextPosition.init)
    }

    func closestPosition(to point: CGPoint, within range: UITextRange) -> UITextPosition? {
        guard let range = range as? JianTextRange, let offset = host.textPosition(at: point) else { return nil }
        return JianTextPosition(min(range.upper, max(range.lower, offset)))
    }

    func characterRange(at point: CGPoint) -> UITextRange? {
        host.textRange(at: point, granularity: Int32(JianTextGranularity_Character.rawValue))
    }

    private var documentLength: UInt32 {
        guard let text = host.documentText() else { return 0 }
        return UInt32(clamping: (text as NSString).length)
    }

    private func positionOffset(_ position: UITextPosition) -> UInt32? {
        (position as? JianTextPosition)?.offset
    }
}

private extension JianRect {
    var cgRect: CGRect {
        CGRect(x: CGFloat(x), y: CGFloat(y), width: CGFloat(width), height: CGFloat(height))
    }
}

private func clampedUInt32(_ value: Int, maximum: UInt32) -> UInt32 {
    guard value != NSNotFound else { return 0 }
    return min(maximum, UInt32(clamping: max(0, value)))
}
