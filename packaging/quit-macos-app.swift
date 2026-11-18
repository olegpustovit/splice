import AppKit

let bundle = URL(fileURLWithPath: CommandLine.arguments[1]).resolvingSymlinksInPath()
let identifier = CommandLine.arguments[2]
let instances = NSRunningApplication.runningApplications(withBundleIdentifier: identifier).filter {
    $0.bundleURL?.resolvingSymlinksInPath().path == bundle.path
}

for instance in instances where !instance.isTerminated {
    guard instance.terminate() || instance.isTerminated else {
        fputs("splice: Could not ask Splice (PID \(instance.processIdentifier)) to quit. Quit it and reinstall.\n", stderr)
        exit(1)
    }
}

let deadline = ProcessInfo.processInfo.systemUptime + 10
while instances.contains(where: { !$0.isTerminated }) {
    guard ProcessInfo.processInfo.systemUptime < deadline else {
        fputs("splice: Splice is still quitting. Wait for it to close and reinstall.\n", stderr)
        exit(1)
    }
    RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
}
