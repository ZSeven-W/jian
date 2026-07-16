import Metal
import QuartzCore
import UIKit

final class JianPlayerView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }

    let host = JianEngineHost()
    private var touchIDs: [ObjectIdentifier: UInt32] = [:]
    private var nextTouchID: UInt32 = 1
    private var keyboardObservers: [NSObjectProtocol] = []
    private var didTearDown = false

    weak var inputDelegate: UITextInputDelegate?
    lazy var tokenizer: UITextInputTokenizer = UITextInputStringTokenizer(textInput: self)
    var markedTextStyle: [NSAttributedString.Key: Any]?
    var selectionAffinity: UITextStorageDirection = .forward

    var autocapitalizationType: UITextAutocapitalizationType = .sentences
    var autocorrectionType: UITextAutocorrectionType = .default
    var spellCheckingType: UITextSpellCheckingType = .default
    var smartQuotesType: UITextSmartQuotesType = .default
    var smartDashesType: UITextSmartDashesType = .default
    var smartInsertDeleteType: UITextSmartInsertDeleteType = .default
    var keyboardType: UIKeyboardType = .default
    var keyboardAppearance: UIKeyboardAppearance = .default
    var returnKeyType: UIReturnKeyType = .default
    var enablesReturnKeyAutomatically = false
    var isSecureTextEntry = false
    var textContentType: UITextContentType?
    var passwordRules: UITextInputPasswordRules?

    private var metalLayer: CAMetalLayer {
        guard let layer = layer as? CAMetalLayer else {
            preconditionFailure("JianPlayerView must be backed by CAMetalLayer")
        }
        return layer
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        commonInit()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        commonInit()
    }

    private func commonInit() {
        isMultipleTouchEnabled = true
        isOpaque = true
        backgroundColor = .black
        contentMode = .redraw
        host.view = self

        let center = NotificationCenter.default
        keyboardObservers.append(center.addObserver(
            forName: UIResponder.keyboardWillChangeFrameNotification,
            object: nil,
            queue: .main
        ) { [weak self] notification in
            self?.keyboardFrameDidChange(notification)
        })
        keyboardObservers.append(center.addObserver(
            forName: UIResponder.keyboardWillHideNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.host.updateKeyboardHeight(0)
        })
    }

    deinit {
        keyboardObservers.forEach(NotificationCenter.default.removeObserver)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard !didTearDown, bounds.width > 0, bounds.height > 0 else { return }
        let surface = metalLayer
        let displayScale = window?.screen.scale ?? UIScreen.main.scale
        contentScaleFactor = displayScale
        surface.contentsScale = displayScale
        surface.device = surface.device ?? MTLCreateSystemDefaultDevice()
        surface.pixelFormat = .bgra8Unorm
        surface.framebufferOnly = false
        surface.presentsWithTransaction = false
        surface.drawableSize = CGSize(
            width: (bounds.width * displayScale).rounded(),
            height: (bounds.height * displayScale).rounded()
        )

        host.configure(surface: surface, logicalSize: bounds.size, scale: displayScale)
        host.updateSafeArea(safeAreaInsets)
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        guard !didTearDown else { return }
        host.updateSafeArea(safeAreaInsets)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil, superview == nil {
            teardownEngine()
        } else {
            setNeedsLayout()
        }
    }

    override var canBecomeFirstResponder: Bool { true }

    func teardownEngine() {
        guard !didTearDown else { return }
        didTearDown = true
        host.teardown()
        keyboardObservers.forEach(NotificationCenter.default.removeObserver)
        keyboardObservers.removeAll()
    }

    func applyFieldConfiguration(_ configuration: JianFieldConfiguration) {
        switch configuration.inputKind {
        case Int32(JianInputKind_Number.rawValue):
            keyboardType = .decimalPad
            isSecureTextEntry = false
        case Int32(JianInputKind_Secure.rawValue):
            keyboardType = .default
            isSecureTextEntry = true
        default:
            keyboardType = .default
            isSecureTextEntry = false
        }

        switch configuration.returnKeyHint {
        case Int32(JianReturnKeyHint_Done.rawValue): returnKeyType = .done
        case Int32(JianReturnKeyHint_Go.rawValue): returnKeyType = .go
        case Int32(JianReturnKeyHint_Next.rawValue): returnKeyType = .next
        case Int32(JianReturnKeyHint_Search.rawValue): returnKeyType = .search
        case Int32(JianReturnKeyHint_Send.rawValue): returnKeyType = .send
        default: returnKeyType = .default
        }
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesBegan(touches, with: event)
        for touch in touches {
            let key = ObjectIdentifier(touch)
            let id = allocateTouchID()
            touchIDs[key] = id
            dispatch(touch, id: id, phase: Int32(JianPointerPhase_Down.rawValue))
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesMoved(touches, with: event)
        for touch in touches {
            guard let id = touchIDs[ObjectIdentifier(touch)] else { continue }
            dispatch(touch, id: id, phase: Int32(JianPointerPhase_Move.rawValue))
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesEnded(touches, with: event)
        finish(touches, phase: Int32(JianPointerPhase_Up.rawValue))
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesCancelled(touches, with: event)
        finish(touches, phase: Int32(JianPointerPhase_Cancel.rawValue))
    }

    private func finish(_ touches: Set<UITouch>, phase: Int32) {
        for touch in touches {
            let key = ObjectIdentifier(touch)
            guard let id = touchIDs.removeValue(forKey: key) else { continue }
            dispatch(touch, id: id, phase: phase)
        }
    }

    private func dispatch(_ touch: UITouch, id: UInt32, phase: Int32) {
        // The engine viewport is bounds.size in logical UIKit points. Metal's
        // contentsScale affects drawable pixels only, so touch points are not scaled.
        host.dispatchPointer(id: id, phase: phase, point: touch.location(in: self))
    }

    private func allocateTouchID() -> UInt32 {
        let candidate = nextTouchID
        nextTouchID = nextTouchID &+ 1
        if nextTouchID == 0 { nextTouchID = 1 }
        return candidate
    }

    private func keyboardFrameDidChange(_ notification: Notification) {
        guard
            let screenFrame = notification.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect,
            let window
        else {
            host.updateKeyboardHeight(0)
            return
        }
        let frameInView = convert(window.convert(screenFrame, from: nil), from: window)
        let intersection = bounds.intersection(frameInView)
        host.updateKeyboardHeight(intersection.isNull ? 0 : intersection.height)
    }
}
