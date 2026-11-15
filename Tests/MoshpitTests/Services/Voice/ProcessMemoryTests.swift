import Foundation
import Testing
@testable import Moshpit

/// The memory figure the voice log lines carry (B677).
@Suite("Process memory")
struct ProcessMemoryTests {
    @Test("The footprint reads, and its peak is never below it")
    func footprint() throws {
        let memory = try #require(ProcessMemory.footprintMB())
        #expect(memory.now > 0)
        #expect(memory.peak >= memory.now)
        #expect(ProcessMemory.logSuffix.hasPrefix(" footprint="))
    }

    @Test("Log durations read as seconds with two decimals")
    func seconds() {
        #expect(WhisperModelStore.seconds(.milliseconds(1_234)) == "1.23")
        #expect(WhisperModelStore.seconds(.seconds(0)) == "0.00")
    }
}
