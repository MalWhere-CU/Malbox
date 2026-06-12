use std::path::Path;

use anyhow::Context;
use phf::phf_map;
use tokio::process::Command;

static PLUGINS: phf::Map<&'static str, &'static str> = phf_map! {
    "psscan" => "windows.psscan.PsScan",
    "pslist" => "windows.pslist.PsList",
    "pstree" => "windows.pstree.PsTree",
    "psxview" => "windows.psxview.PsXView",
    "callbacks" => "windows.callbacks.Callbacks",
    "ssdt" => "windows.ssdt.SSDT",
    "getsids" => "windows.getsids.GetSIDs",
    "privs" => "windows.privileges.Privs",
    "malfind" => "windows.malfind.Malfind",
    "dlllist" => "windows.dlllist.DllList",
    "handles" => "windows.handles.Handles",
    "mutantscan" => "windows.mutantscan.MutantScan",
    "svcscan" => "windows.svcscan.SvcScan",
    "modscan" => "windows.modscan.ModScan",
    "yarascan" => "yarascan.YaraScan",
    "netscan" => "windows.netscan.NetScan",
    "info" => "windows.info.Info",
    "ldrmodules" => "windows.ldrmodules.LdrModules",
    "cmdline" => "windows.cmdline.CmdLine",
    "envars" => "windows.envars.Envars",
    "modules" => "windows.modules.Modules",
    "driverscan" => "windows.driverscan.DriverScan",
    "driverirp" => "windows.driverirp.DriverIrp",
    "verinfo" => "windows.verinfo.VerInfo",
    "filescan" => "windows.filescan.FileScan",
    "vadinfo" => "windows.vadinfo.VadInfo",
    "timers" => "windows.timers.Timers",
    "hivelist" => "windows.registry.hivelist.HiveList",
    "hashdump" => "windows.hashdump.Hashdump",
    "lsadump" => "windows.lsadump.Lsadump",
    "cachedump" => "windows.cachedump.Cachedump",
    "symlinkscan" => "windows.symlinkscan.SymlinkScan",
    "thrdscan" => "windows.thrdscan.ThrdScan",
    "hollowprocesses" => "windows.hollowprocesses.HollowProcesses",
    "processghosting" => "windows.processghosting.ProcessGhosting",
        "suspiciousthreads" =>"windows.suspicious_threads.SuspiciousThreads",
    "devicetree" => "windows.devicetree.DeviceTree",
    "consoles" => "windows.consoles.Consoles",
    "cmdscan" => "windows.cmdscan.CmdScan",
    "amcache" => "windows.amcache.Amcache",
    "shimcache" => "windows.shimcachemem.ShimcacheMem",
    "userassist" => "windows.registry.userassist.UserAssist",
    "unloadedmodules" => "windows.unloadedmodules.UnloadedModules",
    "iat" => "windows.iat.IAT",
    "skeletonkey" => "windows.skeleton_key_check.Skeleton_Key_Check",
    "unhookedsyscalls" => "windows.unhooked_system_calls.unhooked_system_calls",
    "etwpatch" => "windows.etwpatch.EtwPatch",
    "mftscan" => "windows.mftscan.MFTScan",
    "svclist" => "windows.svclist.SvcList",
    "svcdiff" => "windows.svcdiff.SvcDiff",
};
pub async fn run_plugin(dump_path: &Path, plugin: &str) -> anyhow::Result<serde_json::Value> {
    let output = Command::new("vol")
        .arg("-f")
        .arg(dump_path)
        .arg("-r")
        .arg("json")
        .arg(plugin)
        .output()
        .await
        .context(format!("Failed to execute volatility plugin: {}", plugin))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!(
            "Volatility plugin {} failed or returned an error: {}",
            plugin, stderr
        );
        return Ok(serde_json::json!([]));
    }
    let json_str = String::from_utf8_lossy(&output.stdout);
    let parsed =
        serde_json::from_str(&json_str).context("Failed to parse JSON output from plugin: {}")?;
    Ok(parsed)
}

pub async fn analyze_dump(dump_path: &Path) -> anyhow::Result<serde_json::Value> {
    let mut report = serde_json::Map::new();
    for (name, plugin) in PLUGINS.entries() {
        let result = run_plugin(dump_path, plugin).await?;
        report.insert(name.to_string(), result);
    }
    Ok(serde_json::Value::Object(report))
}
