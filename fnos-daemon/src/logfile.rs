//! frpc 日志持久化（2026-08-17 用户反馈：更新重启后内存环形缓冲清空，日志全丢）。
//!
//! 格式：每行一个 Event JSON（与内存历史缓冲一致，read_tail 可直接回灌）。
//! 轮转：超过 max_bytes 时重命名 `<path>.1` 并重建（简单两代，防单文件无限膨胀）。

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 单文件大小上限（超出轮转到 .1）。
const MAX_BYTES: u64 = 1024 * 1024;

pub struct LogFile {
    path: PathBuf,
    file: Mutex<File>,
    max_bytes: u64,
}

impl LogFile {
    /// 在 data_dir/logs/ 下创建/打开 frpc.log（目录不存在则创建）。
    pub fn new(data_dir: &Path) -> std::io::Result<Self> {
        Self::new_with_max(data_dir, MAX_BYTES)
    }

    /// 指定上限创建（测试注入小上限验证轮转）。
    pub fn new_with_max(data_dir: &Path, max_bytes: u64) -> std::io::Result<Self> {
        let dir = data_dir.join("logs");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("frpc.log");
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            file: Mutex::new(file),
            max_bytes,
        })
    }

    /// 追加一行（含换行）；超上限先轮转。
    pub fn append(&self, line: &str) -> std::io::Result<()> {
        let mut f = self.file.lock().map_err(|_| {
            std::io::Error::other("日志文件锁损坏")
        })?;
        if f.metadata().map(|m| m.len()).unwrap_or(0) + line.len() as u64 + 1 > self.max_bytes {
            // 轮转：当前文件 → .1，重建空文件
            drop(f);
            let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
            let mut new_f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            new_f.write_all(line.as_bytes())?;
            new_f.write_all(b"\n")?;
            return Ok(());
        }
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        Ok(())
    }

    /// 读文件尾部最多 n 行（补发/启动回灌用）；轮转的 .1 不合并（只取当前文件尾部）。
    pub fn read_tail(&self, n: usize) -> Vec<String> {
        let Ok(file) = File::open(&self.path) else {
            return Vec::new();
        };
        let mut reader = BufReader::new(file);
        // 直接读全部再取尾 N 行：日志文件上限 1MB，可接受
        let mut lines: Vec<String> = Vec::new();
        let mut buf = String::new();
        loop {
            buf.clear();
            match reader.read_line(&mut buf) {
                Ok(0) => break,
                Ok(_) => lines.push(buf.trim_end().to_string()),
                Err(_) => break,
            }
        }
        lines.into_iter().rev().take(n).rev().collect()
    }
}

/// 测试用：临时目录内的 LogFile。
#[cfg(test)]
pub fn temp_logfile(name: &str) -> (LogFile, PathBuf) {
    let dir = std::env::temp_dir().join(format!("fnos-log-{}-{}", name, std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    (LogFile::new(&dir).expect("创建测试日志文件失败"), dir.join("logs/frpc.log"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 追加与读取尾部() {
        let (log, path) = temp_logfile("append");
        log.append("line1").unwrap();
        log.append("line2").unwrap();
        log.append("line3").unwrap();
        assert_eq!(log.read_tail(2), vec!["line2", "line3"]);
        assert_eq!(log.read_tail(10), vec!["line1", "line2", "line3"]);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn 空文件读取() {
        let (log, path) = temp_logfile("empty");
        assert!(log.read_tail(5).is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn 轮转后新文件继续写() {
        let dir = std::env::temp_dir().join(format!("fnos-log-rot-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let log = LogFile::new_with_max(&dir, 12).expect("创建");
        // 单行 "aaaa\n" = 5 字节：2 行 10 字节，第 3 行 15 字节 > 12 → 触发轮转
        log.append("aaaa").unwrap();
        log.append("bbbb").unwrap();
        log.append("cccc").unwrap();
        // 轮转后当前文件只有最后写入的内容，且 .1 存在
        let tail = log.read_tail(10);
        assert!(!tail.is_empty(), "轮转后应能继续写");
        assert!(dir.join("logs/frpc.log.1").exists(), "旧文件应轮转为 .1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 中文内容往返() {
        let (log, path) = temp_logfile("cn");
        log.append(r#"{"message":"隧道启动成功，日志中文测试"}"#).unwrap();
        let tail = log.read_tail(1);
        assert_eq!(tail.len(), 1);
        assert!(tail[0].contains("中文测试"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
