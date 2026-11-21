import Foundation
import Observation
import SwiftUI
import os

/// What an application's generated bindings provide to the package.
///
/// Each method may throw the bindings' error when the tree refuses a call, for
/// example from the wrong thread or after a panic poisoned it. The package logs
/// the error and goes on as if no tree were attached.
@MainActor
public protocol TreeBridge: AnyObject {
    func pump() throws
    /// Sets the phase: 0 for hidden, 1 for visible and 2 for focused.
    func setPhase(code: Int32) throws
    func back() throws -> Bool
    /// Starts a Back gesture and returns its target's address, or -1.
    func backStart() throws -> Int64
    func backCancel() throws
    func backCommit() throws -> Bool
    func contains(node: Int64) throws -> Bool
    /// Encodes navigation and persistent state. When it throws, the package keeps
    /// the last state it saved.
    func save() throws -> Data
    /// Frees trees whose bindings were released on another thread. The package
    /// calls it on the main thread at every pump, even after the bindings it was
    /// given are gone. Generated bindings export it as a static method.
    static func collectRetired()
}

extension TreeBridge {
    /// Does nothing, for bindings that release their tree only on its own thread.
    public static func collectRetired() {}
}

private let log = Logger(subsystem: "Compono", category: "tree")

/// A child of a router as the UI sees it.
public protocol ComponoChild {
    /// Identifies this entry for as long as it exists.
    var key: Int64 { get }
    /// The component's address.
    var node: Int64 { get }
}

/// A Back gesture in progress.
public struct BackGestureState: Equatable, Sendable {
    /// The component that handles Back if the gesture completes.
    public let target: Int64
    /// How far the gesture has gone, from 0 to 1.
    public internal(set) var progress: CGFloat
}

@Observable
@MainActor
final class NodeVersion {
    var value = 0
}

/// Connects a Rust component tree to SwiftUI.
///
/// A view calls ``observe(_:)`` for each component it reads, so a commit redraws
/// only the views of components that changed.
@Observable
@MainActor
public final class ComponoTree {
    /// Whether a Back press would be handled.
    public private(set) var canBack = false
    /// The Back gesture in progress, if any.
    public private(set) var gesture: BackGestureState?

    @ObservationIgnored private var stacks: [Int64: Int] = [:]
    @ObservationIgnored private var committedTo: Int64?
    /// Weak, because the bindings hold this tree as their listener.
    @ObservationIgnored private weak var bridge: TreeBridge?
    /// Frees trees released on another thread; outlives the bindings.
    @ObservationIgnored private var collectRetired: () -> Void = {}
    @ObservationIgnored private var nodes: [Int64: NodeVersion] = [:]
    /// The phase code of each connected scene that shows this tree.
    @ObservationIgnored private var scenePhases: [UUID: Int32] = [:]
    @ObservationIgnored private var pruneAt = ComponoTree.minimumPruneSize
    @ObservationIgnored private let pumpPending = OSAllocatedUnfairLock(initialState: false)

    private static let minimumPruneSize = 256

    public init() {
        settle = { [weak self] commit, done in
            guard let self else { return done() }
            self.animateSettle(commit: commit, done: done)
        }
    }

    /// Connects the tree's bindings, which may already have committed.
    ///
    /// The tree holds `bridge` weakly, because the bindings hold the tree as their
    /// listener and a strong reference both ways would keep both alive for good.
    /// Whoever creates the bindings keeps them, as ``ComponoRoot`` does.
    public func attach(_ bridge: TreeBridge) {
        self.bridge = bridge
        let bindings = type(of: bridge)
        collectRetired = { bindings.collectRetired() }
    }

    /// Runs `call` on the bindings, or gives `fallback` when none are attached or
    /// the call throws, which is logged.
    private func call<T>(_ name: String, _ fallback: T, _ call: (TreeBridge) throws -> T) -> T {
        guard let bridge else { return fallback }
        do {
            return try call(bridge)
        } catch {
            log.error("\(name, privacy: .public) failed: \(String(describing: error), privacy: .public)")
            return fallback
        }
    }

    /// Records the phase of one scene and gives the tree the highest phase of all
    /// connected scenes.
    ///
    /// The process has one tree, and on iPad each window is a scene with a phase of
    /// its own. The tree is focused while any scene is active, and hidden only once
    /// every scene is in the background.
    func setScenePhase(_ code: Int32, for scene: UUID) {
        scenePhases[scene] = code
        setPhase(scenePhases.values.max() ?? 0)
    }

    /// Forgets a scene that disconnected, so it no longer holds the tree up.
    func removeScene(_ scene: UUID) {
        guard scenePhases.removeValue(forKey: scene) != nil else { return }
        setPhase(scenePhases.values.max() ?? 0)
    }

    /// Makes the calling view depend on one component.
    public func observe(_ node: Int64) {
        if let version = nodes[node] {
            _ = version.value
        } else {
            let version = NodeVersion()
            nodes[node] = version
            _ = version.value
        }
    }

    /// Receives a commit from the bindings' listener, which calls it on the main thread.
    ///
    /// The names match a BoltFFI listener, so `extension ComponoTree: Listener {}`
    /// is all an application writes to receive commits.
    public nonisolated func commit(dirty: [Int64], canBack: Bool) {
        MainActor.assumeIsolated {
            apply(dirty: dirty, canBack: canBack)
        }
    }

    /// Asks for one pump, as a BoltFFI listener's `schedulePump` does. Call from any thread.
    public nonisolated func schedulePump() {
        requestPump()
    }

    private func apply(dirty: [Int64], canBack: Bool) {
        for node in dirty {
            nodes[node]?.value &+= 1
        }
        if self.canBack != canBack {
            self.canBack = canBack
        }
        pruneIfLarge()
    }

    /// Asks for one pump on the main thread. Call from any thread.
    ///
    /// Any number of requests before the pump runs become one.
    public nonisolated func requestPump() {
        let alreadyPending = pumpPending.withLock { pending in
            defer { pending = true }
            return pending
        }
        guard !alreadyPending else { return }
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                self?.runPump()
            }
        }
    }

    private func runPump() {
        pumpPending.withLock { $0 = false }
        call("pump", ()) { try $0.pump() }
        collectRetired()
    }

    /// Delivers a Back press and returns whether it was handled.
    @discardableResult
    public func back() -> Bool {
        call("back", false) { try $0.back() }
    }

    /// Sets the phase: 0 for hidden, 1 for visible and 2 for focused.
    public func setPhase(_ code: Int32) {
        call("setPhase", ()) { try $0.setPhase(code: code) }
    }

    /// Starts a Back gesture. Returns whether anything would handle it.
    public func beginGesture() -> Bool {
        guard bridge != nil else { return false }
        if let previous = gesture {
            // A swipe while the last gesture animates to its end finishes that
            // gesture at once, so the swipe is not lost.
            guard let commit = settlingTo else { return false }
            endGesture(commit: commit, target: previous.target)
        }
        committedTo = nil
        let target = call("backStart", Int64(-1)) { try $0.backStart() }
        guard target != -1 else { return false }
        gestureSerial += 1
        gesture = BackGestureState(target: target, progress: 0)
        return true
    }

    /// Moves the gesture. Progress stays in the UI and never crosses into Rust.
    public func updateGesture(progress: CGFloat) {
        gesture?.progress = min(max(progress, 0), 1)
    }

    /// Ends the gesture, animating a stack to whichever side it ended on.
    ///
    /// `speed` is how fast the finger moved when it let go, in shares of the width
    /// per second; the settling spring starts at that speed.
    public func finishGesture(commit: Bool, speed: CGFloat = 0) {
        guard let target = gesture?.target, settlingTo == nil else { return }
        releaseSpeed = speed
        guard stacks[target] != nil else {
            endGesture(commit: commit, target: target)
            return
        }
        settlingTo = commit
        let serial = gestureSerial
        settle(commit) { [weak self] in
            // A later gesture may have ended this one already.
            guard let self, self.gestureSerial == serial, self.gesture != nil else { return }
            self.endGesture(commit: commit, target: target)
        }
    }

    /// Animates a settling gesture to its end and then calls `done`. Tests replace it.
    @ObservationIgnored var settle: (_ commit: Bool, _ done: @escaping @MainActor () -> Void) -> Void = { _, done in done() }

    @ObservationIgnored private var settlingTo: Bool?
    @ObservationIgnored private var releaseSpeed: CGFloat = 0
    @ObservationIgnored private var gestureSerial = 0

    private func animateSettle(commit: Bool, done: @escaping @MainActor () -> Void) {
        let target: CGFloat = commit ? 1 : 0
        let velocity = settleVelocity(progress: gesture?.progress ?? 0, target: target, speed: releaseSpeed)
        // UIKit's push spring, continuing at the finger's speed, as UIKit does.
        let spring = Animation.interpolatingSpring(platformSpring, initialVelocity: velocity)
        withAnimation(spring, completionCriteria: .logicallyComplete) {
            gesture?.progress = target
        } completion: {
            done()
        }
    }

    private func endGesture(commit: Bool, target: Int64) {
        gesture = nil
        settlingTo = nil
        if commit {
            committedTo = target
            _ = call("backCommit", false) { try $0.backCommit() }
        } else {
            call("backCancel", ()) { try $0.backCancel() }
        }
    }

    /// Returns whether the component at `node` still exists.
    func contains(_ node: Int64) -> Bool {
        call("contains", false) { try $0.contains(node: node) }
    }

    func registerStack(_ owner: Int64) {
        stacks[owner, default: 0] += 1
    }

    func unregisterStack(_ owner: Int64) {
        let remaining = (stacks[owner] ?? 1) - 1
        stacks[owner] = remaining > 0 ? remaining : nil
    }

    /// Returns whether a gesture just completed on this stack, once.
    func takeCommitted(_ owner: Int64) -> Bool {
        guard committedTo == owner else { return false }
        committedTo = nil
        return true
    }

    /// Encodes navigation and persistent state, or returns empty data when the
    /// tree could not be saved, which ``ComponoRoot`` does not write.
    public func save() -> Data {
        call("save", Data()) { try $0.save() }
    }

    /// Forgets destroyed components. Their addresses are never reused as-is.
    private func pruneIfLarge() {
        guard nodes.count > pruneAt, bridge != nil else { return }
        nodes = nodes.filter { node in call("contains", false) { try $0.contains(node: node.key) } }
        pruneAt = max(Self.minimumPruneSize, nodes.count * 2)
    }

    var trackedNodes: Int { nodes.count }
}
