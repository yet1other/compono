import SwiftUI

#if os(iOS)
private let edgeWidth: CGFloat = 24
#endif

private struct ComponoLifecycle: ViewModifier {
    let tree: ComponoTree
    let onBackground: (Data) -> Void
    @Environment(\.scenePhase) private var scenePhase
    /// Names this scene among the others that show the same tree.
    @State private var scene = UUID()

    #if os(iOS)
    @State private var tracking = false
    /// False again after the system cancels a drag, which skips `onEnded`.
    @GestureState private var dragging = false
    @State private var width: CGFloat = 1

    /// Back from the left edge of the screen, as every iOS app has.
    private var edgeSwipe: some Gesture {
        DragGesture(minimumDistance: 10, coordinateSpace: .global)
            .updating($dragging) { _, state, _ in state = true }
            .onChanged { value in
                if !tracking {
                    guard value.startLocation.x <= edgeWidth, value.translation.width > 0, tree.canBack else {
                        return
                    }
                    tracking = tree.beginGesture()
                }
                if tracking {
                    tree.updateGesture(progress: value.translation.width / width)
                }
            }
            .onEnded { value in
                guard tracking else { return }
                tracking = false
                let flung = value.predictedEndTranslation.width / width > 0.6
                let commit = flung || value.translation.width / width > 0.4
                tree.finishGesture(commit: commit, speed: value.velocity.width / width)
            }
    }

    func body(content: Content) -> some View {
        tracked(content)
            .simultaneousGesture(edgeSwipe)
            .onGeometryChange(for: CGFloat.self, of: \.size.width) { width = max($0, 1) }
            .onChange(of: dragging) { _, stillDragging in
                if !stillDragging && tracking {
                    tracking = false
                    tree.finishGesture(commit: false)
                }
            }
    }
    #else
    func body(content: Content) -> some View {
        tracked(content)
    }
    #endif

    /// Reports this scene's phase. The tree takes the highest phase of all scenes,
    /// so one window going to the background does not hide what another shows.
    /// State is saved whenever a scene goes to the background.
    private func tracked(_ content: Content) -> some View {
        content
            .onChange(of: scenePhase, initial: true) { _, phase in
                switch phase {
                case .active:
                    tree.setScenePhase(2, for: scene)
                case .inactive:
                    tree.setScenePhase(1, for: scene)
                case .background:
                    tree.setScenePhase(0, for: scene)
                    onBackground(tree.save())
                @unknown default:
                    tree.setScenePhase(1, for: scene)
                }
            }
            .onDisappear { tree.removeScene(scene) }
    }
}

extension View {
    /// Follows the scene's phase, hands over saved state on entering the background,
    /// and on iOS turns a swipe from the left edge into Back.
    public func componoLifecycle(
        _ tree: ComponoTree,
        onBackground: @escaping (Data) -> Void = { _ in }
    ) -> some View {
        modifier(ComponoLifecycle(tree: tree, onBackground: onBackground))
    }
}
