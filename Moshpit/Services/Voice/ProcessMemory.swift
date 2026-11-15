import Darwin

/// The app's memory as the system judges it (B677): the physical footprint
/// that jetsam measures, now and at its peak since launch, in megabytes. One
/// `task_info` call — cheap enough for a log line after each dictation.
enum ProcessMemory {
    static func footprintMB() -> (now: Int, peak: Int)? {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        guard result == KERN_SUCCESS else { return nil }
        let megabyte = 1 << 20
        let now = Int(info.phys_footprint) / megabyte
        return (now, max(now, Int(info.ledger_phys_footprint_peak) / megabyte))
    }

    /// " footprint=<now>MB peak=<peak>MB", or nothing when the call fails.
    static var logSuffix: String {
        guard let memory = footprintMB() else { return "" }
        return " footprint=\(memory.now)MB peak=\(memory.peak)MB"
    }
}
