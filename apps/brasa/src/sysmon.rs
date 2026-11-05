//! Desempenho do sistema lido de `/proc` (Linux): CPU total e por núcleo, memória, carga e a lista
//! de processos. O desenho segue o "sysinfo" e o "processviewer" do Wave Terminal (Apache-2.0),
//! reescritos em Rust; não há código do Wave aqui.

use std::collections::{HashMap, VecDeque};

/// Amostras guardadas para os gráficos (1 por segundo → dois minutos).
pub const HISTORY: usize = 120;

/// Contadores acumulados de uma linha "cpu…" de `/proc/stat`: (ocioso, total), em jiffies.
fn cpu_counters(line: &str) -> Option<(u64, u64)> {
    let vals: Vec<u64> = line.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
    if vals.len() < 4 {
        return None;
    }
    // idle + iowait contam como ocioso; guest/guest_nice já estão somados em user/nice
    let idle = vals[3] + vals.get(4).copied().unwrap_or(0);
    Some((idle, vals.iter().take(8).sum()))
}

/// Linha "cpu " (total) de `/proc/stat`.
pub fn parse_cpu_line(stat: &str) -> Option<(u64, u64)> {
    stat.lines().find(|l| l.starts_with("cpu ")).and_then(cpu_counters)
}

/// Linhas "cpu0", "cpu1"… de `/proc/stat`, na ordem.
pub fn parse_core_lines(stat: &str) -> Vec<(u64, u64)> {
    stat.lines()
        .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
        .filter_map(cpu_counters)
        .collect()
}

/// Uso entre duas leituras, de 0 a 100.
pub fn usage(prev: (u64, u64), now: (u64, u64)) -> f32 {
    let total = now.1.saturating_sub(prev.1);
    if total == 0 {
        return 0.0;
    }
    let idle = now.0.saturating_sub(prev.0).min(total);
    (total - idle) as f32 * 100.0 / total as f32
}

/// Memória em kB: (total, disponível, swap total, swap livre).
pub fn parse_meminfo(text: &str) -> (u64, u64, u64, u64) {
    let get = |key: &str| {
        text.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };
    (get("MemTotal:"), get("MemAvailable:"), get("SwapTotal:"), get("SwapFree:"))
}

/// Carga média de 1, 5 e 15 minutos.
pub fn parse_loadavg(text: &str) -> (f32, f32, f32) {
    let mut it = text.split_whitespace().filter_map(|v| v.parse::<f32>().ok());
    (it.next().unwrap_or(0.0), it.next().unwrap_or(0.0), it.next().unwrap_or(0.0))
}

/// Campos úteis de `/proc/<pid>/stat`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcStat {
    pub pid: i32,
    pub comm: String,
    pub state: char,
    /// utime + stime, em jiffies
    pub ticks: u64,
    pub threads: i64,
    /// memória residente, em páginas
    pub rss_pages: i64,
}

/// O nome vem entre parênteses e pode ter espaços ou ")"; o resto começa depois do último ")".
pub fn parse_proc_stat(text: &str) -> Option<ProcStat> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let pid = text[..open].trim().parse().ok()?;
    let comm = text[open + 1..close].to_string();
    let rest: Vec<&str> = text[close + 1..].split_whitespace().collect();
    // rest[0] = estado (campo 3); utime = campo 14, stime = 15, threads = 20, rss = 24
    let field = |n: usize| rest.get(n - 3).copied().unwrap_or("0");
    Some(ProcStat {
        pid,
        comm,
        state: field(3).chars().next().unwrap_or('?'),
        ticks: field(14).parse::<u64>().unwrap_or(0) + field(15).parse::<u64>().unwrap_or(0),
        threads: field(20).parse().unwrap_or(0),
        rss_pages: field(24).parse().unwrap_or(0),
    })
}

/// Usuários de `/etc/passwd`: uid → nome.
pub fn parse_passwd(text: &str) -> HashMap<u32, String> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split(':');
            let name = f.next()?;
            let uid = f.nth(1)?.parse().ok()?;
            Some((uid, name.to_string()))
        })
        .collect()
}

pub fn state_label(c: char) -> &'static str {
    match c {
        'R' => "rodando",
        'S' => "dormindo",
        'D' => "disco",
        'Z' => "zumbi",
        'T' | 't' => "parado",
        'I' => "ocioso",
        _ => "?",
    }
}

#[derive(Debug, Clone)]
pub struct ProcInfo {
    pub pid: i32,
    /// nome curto do processo (como no Wave/top)
    pub name: String,
    /// linha de comando completa
    pub command: String,
    pub state: char,
    pub user: String,
    pub threads: i64,
    /// % de um núcleo (100 = um núcleo inteiro), como no top/btop
    pub cpu: f32,
    pub mem_bytes: u64,
    pub mem_pct: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Pid,
    Command,
    User,
    Cpu,
    Mem,
    Threads,
}

/// Ordena a lista; números começam do maior, texto em ordem alfabética.
pub fn sort_procs(procs: &mut [ProcInfo], key: SortKey, desc: bool) {
    procs.sort_by(|a, b| {
        let ord = match key {
            SortKey::Pid => a.pid.cmp(&b.pid),
            SortKey::Command => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::User => a.user.cmp(&b.user),
            SortKey::Cpu => a.cpu.total_cmp(&b.cpu),
            SortKey::Mem => a.mem_bytes.cmp(&b.mem_bytes),
            SortKey::Threads => a.threads.cmp(&b.threads),
        };
        let ord = if desc { ord.reverse() } else { ord };
        ord.then(a.pid.cmp(&b.pid))
    });
}

/// Coletor: guarda a leitura anterior para calcular os deltas e o histórico para os gráficos.
#[derive(Default)]
pub struct Monitor {
    prev_total: Option<(u64, u64)>,
    prev_cores: Vec<(u64, u64)>,
    prev_ticks: HashMap<i32, u64>,
    /// total de jiffies na última varredura de processos (a varredura pode ser mais espaçada que a CPU)
    scan_total: Option<u64>,
    users: HashMap<u32, String>,
    /// comando e usuário de cada pid (não mudam enquanto o processo vive): evita reler 2 arquivos por segundo
    names: HashMap<i32, (String, String)>,
    pub cpu: f32,
    pub cores: Vec<f32>,
    pub cpu_hist: VecDeque<f32>,
    pub mem_hist: VecDeque<f32>,
    pub mem_total_kb: u64,
    pub mem_used_kb: u64,
    pub swap_total_kb: u64,
    pub swap_used_kb: u64,
    pub load: (f32, f32, f32),
    pub procs: Vec<ProcInfo>,
}

const PAGE_KB: u64 = 4;

impl Monitor {
    pub fn new() -> Self {
        let users = std::fs::read_to_string("/etc/passwd").map(|t| parse_passwd(&t)).unwrap_or_default();
        let mut m = Monitor { users, ..Default::default() };
        m.sample(false); // primeira leitura: só a base para os deltas
        m
    }

    /// Nova amostra; `with_procs` também varre os processos (só quando o painel está na tela).
    pub fn sample(&mut self, with_procs: bool) {
        let Ok(stat) = std::fs::read_to_string("/proc/stat") else { return };
        let total = parse_cpu_line(&stat);
        let cores = parse_core_lines(&stat);
        let delta_total = match (self.prev_total, total) {
            (Some(p), Some(n)) => {
                self.cpu = usage(p, n);
                n.1.saturating_sub(p.1)
            }
            _ => 0,
        };
        if self.prev_cores.len() == cores.len() {
            self.cores = self.prev_cores.iter().zip(&cores).map(|(p, n)| usage(*p, *n)).collect();
        }
        self.prev_total = total;
        self.prev_cores = cores;

        let (mt, ma, st, sf) = parse_meminfo(&std::fs::read_to_string("/proc/meminfo").unwrap_or_default());
        self.mem_total_kb = mt;
        self.mem_used_kb = mt.saturating_sub(ma);
        self.swap_total_kb = st;
        self.swap_used_kb = st.saturating_sub(sf);
        self.load = parse_loadavg(&std::fs::read_to_string("/proc/loadavg").unwrap_or_default());

        if delta_total > 0 {
            push_hist(&mut self.cpu_hist, self.cpu);
            push_hist(&mut self.mem_hist, if mt > 0 { self.mem_used_kb as f32 * 100.0 / mt as f32 } else { 0.0 });
        }
        if with_procs {
            // CPU% de cada processo no intervalo desde a varredura anterior (não desde a última amostra)
            let now = total.map(|t| t.1).unwrap_or(0);
            let since_scan = self.scan_total.map_or(delta_total, |prev| now.saturating_sub(prev));
            self.scan_total = Some(now);
            self.scan_procs(since_scan);
        }
    }

    fn scan_procs(&mut self, delta_total: u64) {
        let ncpu = self.prev_cores.len().max(1) as f32;
        // jiffies de um núcleo no intervalo
        let per_core = delta_total as f32 / ncpu;
        let mut ticks = HashMap::new();
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir("/proc") else { return };
        for e in dir.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else { continue };
            let Some(st) = std::fs::read_to_string(e.path().join("stat")).ok().and_then(|t| parse_proc_stat(&t)) else { continue };
            let cpu = match self.prev_ticks.get(&pid) {
                Some(prev) if per_core > 0.0 => st.ticks.saturating_sub(*prev) as f32 * 100.0 / per_core,
                _ => 0.0,
            };
            let known = self.prev_ticks.contains_key(&pid);
            ticks.insert(pid, st.ticks);
            if !known || !self.names.contains_key(&pid) {
                let uid = std::fs::read_to_string(e.path().join("status"))
                    .ok()
                    .and_then(|t| t.lines().find(|l| l.starts_with("Uid:")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u32>().ok()));
                let user = uid.map(|u| self.users.get(&u).cloned().unwrap_or_else(|| u.to_string())).unwrap_or_default();
                // linha de comando completa quando existe (threads do kernel não têm)
                let cmdline = std::fs::read(e.path().join("cmdline")).ok().map(|b| String::from_utf8_lossy(&b).replace('\0', " ").trim().to_string());
                let command = cmdline.filter(|c| !c.is_empty()).unwrap_or_else(|| format!("[{}]", st.comm));
                self.names.insert(pid, (command, user));
            }
            let (command, user) = self.names.get(&pid).cloned().unwrap_or_default();
            let mem_kb = st.rss_pages.max(0) as u64 * PAGE_KB;
            out.push(ProcInfo {
                pid,
                name: st.comm.clone(),
                command,
                state: st.state,
                user,
                threads: st.threads,
                cpu,
                mem_bytes: mem_kb * 1024,
                mem_pct: if self.mem_total_kb > 0 { mem_kb as f32 * 100.0 / self.mem_total_kb as f32 } else { 0.0 },
            });
        }
        // pids que sumiram saem do cache (um pid novo com o mesmo número relê os nomes)
        self.names.retain(|pid, _| ticks.contains_key(pid));
        self.prev_ticks = ticks;
        self.procs = out;
    }
}

fn push_hist(h: &mut VecDeque<f32>, v: f32) {
    if h.len() == HISTORY {
        h.pop_front();
    }
    h.push_back(v);
}

/// "1,2 GiB", "512 MiB", "800 KiB".
pub fn human_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GiB", b / 1024f64.powi(3))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.0} MiB", b / 1024f64.powi(2))
    } else {
        format!("{:.0} KiB", b / 1024.0)
    }
}

/// Tamanho curto como no Wave: "1.22G", "31.7M", "512K", "0.00B".
pub fn short_bytes(b: u64) -> String {
    let units = [("G", 1u64 << 30), ("M", 1 << 20), ("K", 1 << 10)];
    for (u, size) in units {
        if b >= size {
            let v = b as f64 / size as f64;
            return if v >= 100.0 { format!("{v:.0}{u}") } else if v >= 10.0 { format!("{v:.1}{u}") } else { format!("{v:.2}{u}") };
        }
    }
    format!("{b:.2}B", b = b as f64)
}

/// Envia um sinal a um processo com o `kill` do sistema (TERM, INT, HUP, KILL...).
pub fn send_signal(pid: i32, signal: &str) -> Result<(), String> {
    let out = std::process::Command::new("kill").arg(format!("-{signal}")).arg(pid.to_string()).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proc_stat_and_computes_usage() {
        let a = parse_cpu_line("cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 1 2 3 4\n").unwrap();
        assert_eq!(a, (800, 1000));
        let b = parse_cpu_line("cpu  200 0 200 1300 200 0 0 0 0 0\n").unwrap();
        // +200 ocupado, +700 ocioso → 200/900
        assert!((usage(a, b) - 22.22).abs() < 0.1);
        assert_eq!(usage(a, a), 0.0);
        assert_eq!(parse_cpu_line("intr 1 2"), None);
    }

    #[test]
    fn cores_meminfo_and_load() {
        let stat = "cpu  1 1 1 1\ncpu0 1 0 0 9\ncpu1 5 0 0 5\nintr 3\n";
        assert_eq!(parse_core_lines(stat), [(9, 10), (5, 10)]);
        let mem = "MemTotal:  1000 kB\nMemFree: 100 kB\nMemAvailable: 400 kB\nSwapTotal: 50 kB\nSwapFree: 20 kB\n";
        assert_eq!(parse_meminfo(mem), (1000, 400, 50, 20));
        assert_eq!(parse_loadavg("2.67 2.97 3.18 1/2007 390067"), (2.67, 2.97, 3.18));
    }

    #[test]
    fn proc_stat_with_spaces_and_parens_in_name() {
        let t = "1234 (Web Content (x)) S 1 1234 1 0 -1 0 0 0 0 0 150 50 0 0 20 0 7 0 100 1000 2048 0";
        let p = parse_proc_stat(t).unwrap();
        assert_eq!((p.pid, p.comm.as_str(), p.state, p.ticks, p.threads, p.rss_pages), (1234, "Web Content (x)", 'S', 200, 7, 2048));
        assert_eq!(parse_passwd("root:x:0:0::/root:/bin/bash\nana:x:1000:1000::/home/ana:/bin/zsh\n").get(&1000).map(String::as_str), Some("ana"));
    }

    #[test]
    fn sorting_and_sizes() {
        let p = |pid, cpu, mem| ProcInfo { pid, name: format!("p{pid}"), command: format!("p{pid}"), state: 'S', user: "u".into(), threads: 1, cpu, mem_bytes: mem, mem_pct: 0.0 };
        let mut v = vec![p(3, 1.0, 10), p(1, 9.0, 5), p(2, 5.0, 50)];
        sort_procs(&mut v, SortKey::Cpu, true);
        assert_eq!(v.iter().map(|x| x.pid).collect::<Vec<_>>(), [1, 2, 3]);
        sort_procs(&mut v, SortKey::Mem, true);
        assert_eq!(v[0].pid, 2);
        sort_procs(&mut v, SortKey::Pid, false);
        assert_eq!(v[0].pid, 1);
        assert_eq!(human_bytes(1536 * 1024 * 1024), "1.5 GiB");
        assert_eq!(human_bytes(512 * 1024 * 1024), "512 MiB");
        assert_eq!(state_label('R'), "rodando");
        assert_eq!(short_bytes(1_310_000_000), "1.22G");
        assert_eq!(short_bytes(33_240_000), "31.7M");
        assert_eq!(short_bytes(0), "0.00B");
    }

    #[test]
    fn real_proc_scan_finds_this_process() {
        let mut m = Monitor::new();
        m.sample(true);
        let me = std::process::id() as i32;
        assert!(m.procs.iter().any(|p| p.pid == me));
        assert!(m.mem_total_kb > 0);
        assert!(!m.prev_cores.is_empty());
    }
}
