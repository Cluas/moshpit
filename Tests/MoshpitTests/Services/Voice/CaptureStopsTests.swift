import AVFoundation
import Foundation
import Testing
@testable import Moshpit

/// What ends a dictation's capture from outside the app (B674).
@Suite("Capture stops")
struct CaptureStopsTests {
    final class Count: @unchecked Sendable {
        private let lock = NSLock()
        private var value = 0
        func bump() { lock.withLock { value += 1 } }
        var count: Int { lock.withLock { value } }
    }

    private func watch(running: Bool = false) -> (NotificationCenter, AVAudioSession, AVAudioEngine, Count, [NSObjectProtocol]) {
        let center = NotificationCenter()
        let session = AVAudioSession.sharedInstance()
        let engine = AVAudioEngine()
        let stops = Count()
        let observers = CaptureStops.observe(center, session: session, engine: engine,
                                             engineRunning: { running }, onStop: { stops.bump() })
        return (center, session, engine, stops, observers)
    }

    @Test("A new audio route stops the engine: the capture ends as an interruption does (B674)")
    func configurationChange() {
        let (center, _, engine, stops, observers) = watch()
        center.post(name: .AVAudioEngineConfigurationChange, object: engine)
        #expect(stops.count == 1)
        observers.forEach(center.removeObserver)
    }

    @Test("A change the engine rode out, or another engine's, ends nothing")
    func configurationChangeIgnored() {
        let (center, _, engine, stops, observers) = watch(running: true)
        center.post(name: .AVAudioEngineConfigurationChange, object: engine)
        center.post(name: .AVAudioEngineConfigurationChange, object: AVAudioEngine())
        #expect(stops.count == 0)
        observers.forEach(center.removeObserver)
    }

    @Test("An interruption's beginning ends the capture; its end doesn't")
    func interruption() {
        let (center, session, _, stops, observers) = watch()
        center.post(name: AVAudioSession.interruptionNotification, object: session,
                    userInfo: [AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.began.rawValue])
        center.post(name: AVAudioSession.interruptionNotification, object: session,
                    userInfo: [AVAudioSessionInterruptionTypeKey: AVAudioSession.InterruptionType.ended.rawValue])
        #expect(stops.count == 1)
        observers.forEach(center.removeObserver)
    }

    @Test("The media services resetting ends the capture")
    func mediaServicesReset() {
        let (center, session, _, stops, observers) = watch()
        center.post(name: AVAudioSession.mediaServicesWereResetNotification, object: session)
        #expect(stops.count == 1)
        observers.forEach(center.removeObserver)
    }
}
