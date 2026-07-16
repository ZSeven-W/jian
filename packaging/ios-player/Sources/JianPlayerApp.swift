import SwiftUI

@main
struct JianPlayerApp: App {
    var body: some Scene {
        WindowGroup {
            JianPlayerContainer()
                .ignoresSafeArea()
        }
    }
}

private struct JianPlayerContainer: UIViewRepresentable {
    func makeUIView(context: Context) -> JianPlayerView {
        JianPlayerView(frame: .zero)
    }

    func updateUIView(_ view: JianPlayerView, context: Context) {
        view.setNeedsLayout()
    }

    static func dismantleUIView(_ view: JianPlayerView, coordinator: ()) {
        view.teardownEngine()
    }
}
