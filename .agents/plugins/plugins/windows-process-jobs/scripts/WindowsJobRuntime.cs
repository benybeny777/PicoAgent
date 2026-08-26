using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

public sealed class WindowsJobOutcome
{
    public int ExitCode { get; set; }
    public bool Cancelled { get; set; }
}

public sealed class ManagedWindowsJob : IDisposable
{
    const uint JobObjectExtendedLimitInformation = 9;
    const uint JobObjectLimitKillOnJobClose = 0x00002000;

    [StructLayout(LayoutKind.Sequential)]
    struct IoCounters
    {
        public ulong ReadOperationCount;
        public ulong WriteOperationCount;
        public ulong OtherOperationCount;
        public ulong ReadTransferCount;
        public ulong WriteTransferCount;
        public ulong OtherTransferCount;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct BasicLimitInformation
    {
        public long PerProcessUserTimeLimit;
        public long PerJobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass;
        public uint SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct ExtendedLimitInformation
    {
        public BasicLimitInformation BasicLimitInformation;
        public IoCounters IoInfo;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr CreateJobObject(IntPtr attributes, string name);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool SetInformationJobObject(
        IntPtr job,
        uint infoClass,
        IntPtr info,
        uint length);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

    [DllImport("kernel32.dll", SetLastError = true)]
    static extern bool TerminateJobObject(IntPtr job, uint exitCode);

    [DllImport("kernel32.dll")]
    static extern bool CloseHandle(IntPtr handle);

    readonly IntPtr jobHandle;
    readonly Process process;
    readonly Task stdoutPump;
    readonly Task stderrPump;
    readonly BoundedFileLogger stdoutLog;
    readonly BoundedFileLogger stderrLog;

    public int ProcessId { get { return process.Id; } }
    public string ProcessStartTimeUtc { get { return process.StartTime.ToUniversalTime().ToString("o"); } }

    ManagedWindowsJob(
        IntPtr jobHandle,
        Process process,
        Task stdoutPump,
        Task stderrPump,
        BoundedFileLogger stdoutLog,
        BoundedFileLogger stderrLog)
    {
        this.jobHandle = jobHandle;
        this.process = process;
        this.stdoutPump = stdoutPump;
        this.stderrPump = stderrPump;
        this.stdoutLog = stdoutLog;
        this.stderrLog = stderrLog;
    }

    public static ManagedWindowsJob Start(
        string pwshPath,
        string payloadScript,
        string jobDirectory,
        string stdoutPath,
        string stderrPath,
        long maxLogBytes)
    {
        IntPtr job = CreateJobObject(IntPtr.Zero, null);
        if (job == IntPtr.Zero)
            throw new InvalidOperationException("CreateJobObject failed: " + Marshal.GetLastWin32Error());

        var limits = new ExtendedLimitInformation();
        limits.BasicLimitInformation.LimitFlags = JobObjectLimitKillOnJobClose;
        int size = Marshal.SizeOf(typeof(ExtendedLimitInformation));
        IntPtr buffer = Marshal.AllocHGlobal(size);
        try
        {
            Marshal.StructureToPtr(limits, buffer, false);
            if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, buffer, (uint)size))
                throw new InvalidOperationException("SetInformationJobObject failed: " + Marshal.GetLastWin32Error());
        }
        catch
        {
            CloseHandle(job);
            throw;
        }
        finally
        {
            Marshal.FreeHGlobal(buffer);
        }

        var start = new ProcessStartInfo(pwshPath)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            WorkingDirectory = jobDirectory,
        };
        start.ArgumentList.Add("-NoProfile");
        start.ArgumentList.Add("-File");
        start.ArgumentList.Add(payloadScript);
        start.ArgumentList.Add("--job-dir");
        start.ArgumentList.Add(jobDirectory);

        var child = new Process { StartInfo = start };
        try
        {
            if (!child.Start())
                throw new InvalidOperationException("payload process did not start");
            if (!AssignProcessToJobObject(job, child.Handle))
                throw new InvalidOperationException("AssignProcessToJobObject failed: " + Marshal.GetLastWin32Error());

            var outLog = new BoundedFileLogger(stdoutPath, maxLogBytes);
            var errLog = new BoundedFileLogger(stderrPath, maxLogBytes);
            Task outTask = PumpAsync(child.StandardOutput, outLog);
            Task errTask = PumpAsync(child.StandardError, errLog);
            File.WriteAllText(Path.Combine(jobDirectory, "go.signal"), "go", new UTF8Encoding(false));
            return new ManagedWindowsJob(job, child, outTask, errTask, outLog, errLog);
        }
        catch
        {
            try { if (!child.HasExited) child.Kill(true); } catch { }
            child.Dispose();
            CloseHandle(job);
            throw;
        }
    }

    static async Task PumpAsync(StreamReader reader, BoundedFileLogger logger)
    {
        char[] buffer = new char[8192];
        while (true)
        {
            int count = await reader.ReadAsync(buffer, 0, buffer.Length).ConfigureAwait(false);
            if (count == 0) break;
            logger.Append(new string(buffer, 0, count));
        }
    }

    public bool WaitForExit(int milliseconds)
    {
        return process.WaitForExit(milliseconds);
    }

    public void RequestCancel(int graceMilliseconds)
    {
        if (process.HasExited) return;
        try { process.CloseMainWindow(); } catch { }
        if (!process.WaitForExit(graceMilliseconds))
        {
            if (!TerminateJobObject(jobHandle, 1223))
                throw new InvalidOperationException("TerminateJobObject failed: " + Marshal.GetLastWin32Error());
            process.WaitForExit(30000);
        }
    }

    public WindowsJobOutcome Finish(bool cancelled)
    {
        process.WaitForExit();
        Task.WaitAll(new[] { stdoutPump, stderrPump }, 30000);
        stdoutLog.TrimToLimit();
        stderrLog.TrimToLimit();
        return new WindowsJobOutcome { ExitCode = process.ExitCode, Cancelled = cancelled };
    }

    public void Dispose()
    {
        process.Dispose();
        CloseHandle(jobHandle);
    }
}

sealed class BoundedFileLogger
{
    readonly string path;
    readonly long maxBytes;
    readonly object gate = new object();
    readonly UTF8Encoding utf8 = new UTF8Encoding(false);

    public BoundedFileLogger(string path, long maxBytes)
    {
        this.path = path;
        this.maxBytes = maxBytes;
    }

    public void Append(string text)
    {
        lock (gate)
        {
            byte[] incoming = utf8.GetBytes(text);
            using (var stream = new FileStream(path, FileMode.Append, FileAccess.Write, FileShare.Read))
                stream.Write(incoming, 0, incoming.Length);
            var file = new FileInfo(path);
            if (file.Length <= maxBytes + 1024 * 1024) return;
            Trim();
        }
    }

    public void TrimToLimit()
    {
        lock (gate)
        {
            var file = new FileInfo(path);
            if (!file.Exists || file.Length <= maxBytes) return;
            Trim();
        }
    }

    void Trim()
    {
            byte[] all = File.ReadAllBytes(path);
            int keep = (int)Math.Min(maxBytes * 3 / 4, all.LongLength);
            byte[] marker = utf8.GetBytes("\n[古いログを容量上限のため省略]\n");
            using (var stream = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.Read))
            {
                stream.Write(marker, 0, marker.Length);
                stream.Write(all, all.Length - keep, keep);
            }
    }
}
