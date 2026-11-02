//! O processo de verdade, pelo socket, como um cliente.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const RMS1D: &str = env!("CARGO_BIN_EXE_rms1d");
const ROTEIRO: &str = env!("CARGO_BIN_EXE_rms1d-roteiro");

/// Curto de propósito: caminho de socket tem limite de tamanho.
fn pasta(nome: &str) -> PathBuf {
    let d = PathBuf::from(format!("/tmp/rms1-{}-{nome}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Processo {
    filho: Child,
    socket: PathBuf,
}

impl Drop for Processo {
    fn drop(&mut self) {
        let _ = self.filho.kill();
        let _ = self.filho.wait();
    }
}

fn subir(d: &Path, toml: &str) -> Processo {
    let cfg = d.join("rms1d.toml");
    std::fs::write(&cfg, toml).unwrap();
    let socket = d.join("r.sock");
    let filho = Command::new(RMS1D)
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--socket",
            socket.to_str().unwrap(),
        ])
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let inicio = Instant::now();
    while UnixStream::connect(&socket).is_err() {
        assert!(inicio.elapsed() < Duration::from_secs(5), "não subiu");
        std::thread::sleep(Duration::from_millis(20));
    }
    Processo { filho, socket }
}

struct Cliente(UnixStream, BufReader<UnixStream>, u64);

impl Cliente {
    fn novo(s: &Path) -> Self {
        let f = UnixStream::connect(s).unwrap();
        f.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let l = BufReader::new(f.try_clone().unwrap());
        Self(f, l, 1)
    }

    fn chamar(&mut self, m: &str, p: Value) -> Value {
        let id = self.2;
        self.2 += 1;
        writeln!(
            self.0,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":m,"params":p})
        )
        .unwrap();
        loop {
            let mut l = String::new();
            self.1.read_line(&mut l).unwrap();
            let v: Value = serde_json::from_str(&l).unwrap();
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    /// O próximo quadro de estado.
    fn estado(&mut self) -> Value {
        loop {
            let mut l = String::new();
            self.1.read_line(&mut l).unwrap();
            let v: Value = serde_json::from_str(&l).unwrap();
            if v["method"] == "robot.state" {
                return v["params"].clone();
            }
        }
    }
}

const FALSO: &str = "[corpo]\ntipo = \"falso\"\n";

/// O que o console chama ao abrir: nada pode voltar erro, ou a página mostra alerta vermelho.
#[test]
fn tudo_que_o_console_chama_ao_abrir_responde_sem_erro() {
    let d = pasta("console");
    let p = subir(&d, FALSO);
    let mut c = Cliente::novo(&p.socket);
    std::thread::sleep(Duration::from_millis(1200));
    for (m, params) in [
        ("hello", json!({"api_version": 0})),
        ("system.info", json!({})),
        ("telemetry.status", json!({})),
        ("robot.subscribe", json!({"hz": 2})),
        ("robot.policies", json!({})),
        ("robot.mode", json!({})),
        ("robot.health", json!({})),
    ] {
        let r = c.chamar(m, params);
        assert!(r["error"].is_null(), "{m}: {r}");
    }
    let mut c = Cliente::novo(&p.socket);
    let s = c.chamar("robot.policies", json!({}));
    assert_eq!(
        s["result"]["skills"],
        json!(["modo_livre", "chassi_segue_torre", "torre_segue_chassi"])
    );
    let i = c.chamar("system.info", json!({}));
    assert_eq!(i["result"]["simulated"], true);
    let h = c.chamar("robot.health", json!({}));
    assert_eq!(h["result"]["healthy"], true, "{h}");
    assert!(
        h["result"]["control_loop"]["achieved_hz"].as_f64().unwrap() > 40.0,
        "{h}"
    );
}

/// Um valor no arquivo muda o comportamento: é o prazo, medido pelo socket.
#[test]
fn o_prazo_de_rms1d_toml_muda_o_comportamento() {
    fn tempo_ate_deadman(toml: &str, nome: &str) -> Duration {
        let d = pasta(nome);
        let p = subir(&d, toml);
        let mut a = Cliente::novo(&p.socket);
        a.chamar("robot.subscribe", json!({}));
        let mut c = Cliente::novo(&p.socket);
        c.chamar("robot.enable", json!({"on": true}));
        c.chamar("robot.move", json!({"vx": 0.1, "vy": 0.0, "vyaw": 0.0}));
        let t = Instant::now();
        loop {
            let e = a.estado();
            if e["move"]["limited_by"] == json!(["deadman"]) {
                return t.elapsed();
            }
            assert!(t.elapsed() < Duration::from_secs(3), "{e}");
        }
    }
    let curto = tempo_ate_deadman(
        "[corpo]\ntipo = \"falso\"\n[seguranca]\nprazo_ms = 100\n",
        "pc",
    );
    let longo = tempo_ate_deadman(FALSO, "pl");
    assert!(curto < Duration::from_millis(350), "{curto:?}");
    assert!(longo >= Duration::from_millis(480), "{longo:?}");
}

/// Chave errada não sobe, e diz qual é.
#[test]
fn chave_desconhecida_nao_sobe_e_diz_o_nome() {
    let d = pasta("chave");
    let cfg = d.join("rms1d.toml");
    std::fs::write(&cfg, "[seguranca]\nprazzo_ms = 100\n").unwrap();
    let saida = Command::new(RMS1D)
        .args([
            "--config",
            cfg.to_str().unwrap(),
            "--socket",
            d.join("r.sock").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!saida.status.success());
    let erro = String::from_utf8_lossy(&saida.stderr);
    assert!(erro.contains("prazzo_ms"), "{erro}");
}

/// Um só dono por socket, e a trava nunca é apagada.
#[test]
fn um_segundo_processo_e_recusado_e_a_trava_fica() {
    let d = pasta("dono");
    let p = subir(&d, FALSO);
    let saida = Command::new(RMS1D)
        .args([
            "--config",
            d.join("rms1d.toml").to_str().unwrap(),
            "--socket",
            p.socket.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!saida.status.success());
    assert!(String::from_utf8_lossy(&saida.stderr).contains("dono"));
    let trava = d.join("r.sock.lock");
    assert!(trava.exists());
    drop(p);
    assert!(trava.exists(), "a trava nunca é apagada");
    subir(&d, FALSO);
}

/// Os modos pelo caminho do console: `robot.do`, e o estado responde.
#[test]
fn skill_de_modo_troca_o_modo_e_nome_errado_e_recusado() {
    let d = pasta("modo");
    let p = subir(&d, FALSO);
    let mut a = Cliente::novo(&p.socket);
    a.chamar("robot.subscribe", json!({}));
    let mut c = Cliente::novo(&p.socket);
    c.chamar("robot.enable", json!({"on": true}));
    for m in ["chassi_segue_torre", "torre_segue_chassi", "modo_livre"] {
        let r = c.chamar("robot.do", json!({"skill": m}));
        assert_eq!(r["result"]["accepted"], true, "{r}");
        let t = Instant::now();
        loop {
            if a.estado()["policy"] == m {
                break;
            }
            assert!(t.elapsed() < Duration::from_secs(2), "{m} não apareceu");
        }
    }
    let r = c.chamar("robot.do", json!({"skill": "polite-bow"}));
    assert_eq!(r["result"]["accepted"], false, "{r}");
}

/// O roteiro completo, contra o corpo falso, duas vezes: mesmos quadros, fora o tempo. É a mesma prova
/// que compara simulador e robô.
#[test]
fn o_roteiro_passa_e_duas_execucoes_mandam_os_mesmos_quadros() {
    let d = pasta("rot");
    let mut gravacoes = Vec::new();
    for n in ["a", "b"] {
        let g = d.join(format!("{n}.quadros"));
        let toml = format!("[corpo]\ntipo = \"falso\"\ngravar = \"{}\"\n", g.display());
        let sub = d.join(n);
        std::fs::create_dir_all(&sub).unwrap();
        let mut p = subir(&sub, &toml);
        let s = Command::new(ROTEIRO)
            .args(["--socket", p.socket.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(s.status.success(), "{}", String::from_utf8_lossy(&s.stderr));
        // SIGTERM para descarregar a gravação.
        let _ = Command::new("kill").arg(p.filho.id().to_string()).status();
        let _ = p.filho.wait();
        gravacoes.push(g);
    }
    let s = Command::new(ROTEIRO)
        .arg("--comparar")
        .args(&gravacoes)
        .output()
        .unwrap();
    assert!(
        s.status.success(),
        "{}{}",
        String::from_utf8_lossy(&s.stdout),
        String::from_utf8_lossy(&s.stderr)
    );
}
