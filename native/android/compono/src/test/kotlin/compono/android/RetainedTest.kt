package compono.android

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

/** Bindings that fail once closed, as the generated ones do. */
private class ClosingApp : TreeBridge, AutoCloseable {
    var closed = false
    var pumps = 0

    private fun check() = check(!closed) { "ShowcaseApp is closed" }

    override fun pump() {
        check()
        pumps += 1
    }

    override fun setPhase(code: Int) = check()

    override fun back(): Boolean = check().let { false }

    override fun backStart(): Long = check().let { -1L }

    override fun backCancel() = check()

    override fun backCommit(): Boolean = check().let { false }

    override fun contains(node: Long): Boolean = check().let { false }

    override fun save(): ByteArray = check().let { ByteArray(0) }

    override fun close() {
        closed = true
    }
}

class RetainedTest {
    @Test
    fun aFailedSaveKeepsTheLastGoodState() {
        // 2-1: a poisoned tree saves nothing, and its empty bytes replace nothing.
        val good = byteArrayOf(1, 2, 3)
        assertArrayEquals(good, lastGoodState(good, ByteArray(0)))
        val newer = byteArrayOf(4)
        assertArrayEquals(newer, lastGoodState(good, newer))
    }

    @Test
    fun aPumpScheduledBeforeOnClearedDoesNothing() {
        // 6-1: the Activity finishes while a pump is already posted.
        val queue = ArrayDeque<Runnable>()
        val tree = ComponoTree(post = { queue.addLast(it) })
        val app = ClosingApp()
        tree.attach(app)
        val store = ViewModelStore()
        val factory = object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <V : ViewModel> create(modelClass: Class<V>): V = RetainedRoot() as V
        }
        val holder = ViewModelProvider(store, factory)["compono", RetainedRoot::class.java]
        holder.root = ComponoRoot(tree, app)

        tree.schedulePump()
        store.clear()
        assertEquals(true, app.closed)
        while (queue.isNotEmpty()) queue.removeFirst().run()
        assertEquals(0, app.pumps)
        // Nothing reaches the closed bindings afterwards either.
        tree.schedulePump()
        while (queue.isNotEmpty()) queue.removeFirst().run()
        assertEquals(false, tree.back())
        assertEquals(0, tree.save().size)
    }

    @Test
    fun aThrownErrorIsReportedAndAnsweredAsNoTree() {
        // 6-3: the kind of a NativeError arrives as an exception, and nothing crashes.
        val reported = mutableListOf<String>()
        reportFailure = { message, _ -> reported += message }
        val queue = ArrayDeque<Runnable>()
        val tree = ComponoTree(post = { queue.addLast(it) })
        val app = ClosingApp().also { it.close() }
        tree.attach(app)
        assertEquals(false, tree.back())
        assertEquals(false, tree.beginGesture())
        assertEquals(0, tree.save().size)
        tree.requestPump()
        while (queue.isNotEmpty()) queue.removeFirst().run()
        assertEquals(listOf("back failed", "backStart failed", "save failed", "pump failed"), reported)
    }

    @Test
    fun aThrowingCommitIsReportedInsteadOfLost() {
        // 6-5: the JNI glue would hide it; the library catches and logs it.
        val reported = mutableListOf<String>()
        reportFailure = { message, _ -> reported += message }
        val tree = ComponoTree(post = { it.run() })
        val app = ClosingApp().also { it.close() }
        tree.attach(app)
        // Enough tracked nodes to make the commit prune, which asks the closed app.
        repeat(300) { tree.observe(it.toLong()) }
        tree.commit(LongArray(0), canBack = false)
        assertEquals(true, reported.all { it == "contains failed" })
        assertEquals(true, reported.isNotEmpty())
    }

    @Test
    fun everyPumpCollectsRetiredTreesEvenAfterDetach() {
        // 6-4: trees released on another thread are freed on the main thread.
        var collected = 0
        val queue = ArrayDeque<Runnable>()
        val tree = ComponoTree(post = { queue.addLast(it) })
        tree.attach(
            treeBridge({}, {}, { false }, { -1L }, {}, { false }, { false }, { ByteArray(0) },
                collectRetired = { collected += 1 }),
        )
        tree.requestPump()
        while (queue.isNotEmpty()) queue.removeFirst().run()
        assertEquals(1, collected)
        tree.detach()
        tree.requestPump()
        while (queue.isNotEmpty()) queue.removeFirst().run()
        assertEquals(2, collected)
    }
}
