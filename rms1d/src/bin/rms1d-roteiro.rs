//! O mesmo roteiro no simulador e no robô, pelo socket do contrato, com as chamadas que a inteligência
//! fará. Não há caminho paralelo: este programa é só mais um cliente.
//!
//! Sai com erro no primeiro passo que reprovar, e diz qual. `--comparar a b` compara duas gravações de
//! quadros, para provar que simulador e robô receberam o mesmo.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::Parser;
use duck_ipc_proto as proto;
use serde_json::{Value, json};

#[derive(Parser, Debug)]
#[command(about = "O roteiro de prova do rms1d, pelo socket do contrato")]
struct Args {
    #[arg(long, default_value = proto::socket::ROBOT)]
    socket: PathBuf,
    /// Velocidade do passo de movimento, em m/s. Nas rodas fora do chão, 0,3.
    #[arg(long, default_value_t = 0.3)]
    velocidade: f64,
    /// Só o passo de andar reto por 1 s, para o chão, armando e desarmando.
    #[arg(long)]
    so_andar: bool,
    /// Não testa nada no robô: compara duas gravações de quadros.
    #[arg(long, num_args = 2, value_names = ["A", "B"])]
    comparar: Vec<PathBuf>,
}

struct Cliente {
    fluxo: UnixStream,
    leitor: BufReader<UnixStream>,
    proximo: u64,
}

impl Cliente {
    fn conectar(socket: &std::path::Path) -> std::io::Result<Self> {
        let fluxo = UnixStream::connect(socket)?;
        fluxo.set_read_timeout(Some(Duration::from_secs(5)))?;
        let leitor = BufReader::new(fluxo.try_clone()?);
        Ok(Self {
            fluxo,
            leitor,
            proximo: 1,
        })
    }

    fn chamar(&mut self, metodo: &str, params: Value) -> Result<Value, String> {
        let id = self.proximo;
        self.proximo += 1;
        let linha = json!({"jsonrpc": "2.0", "id": id, "method": metodo, "params": params});
        writeln!(self.fluxo, "{linha}").map_err(|e| e.to_string())?;
        loop {
            let mut l = String::new();
            self.leitor.read_line(&mut l).map_err(|e| e.to_string())?;
            if l.is_empty() {
                return Err("o socket fechou".to_owned());
            }
            let v: Value = serde_json::from_str(&l).map_err(|e| e.to_string())?;
            if v["id"] == json!(id) {
                if !v["error"].is_null() {
                    return Err(format!("{metodo}: {}", v["error"]));
                }
                return Ok(v["result"].clone());
            }
        }
    }

    fn notificar(&mut self, metodo: &str, params: Value) -> Result<(), String> {
        let linha = json!({"jsonrpc": "2.0", "method": metodo, "params": params});
        writeln!(self.fluxo, "{linha}").map_err(|e| e.to_string())
    }
}

/// O último `robot.state`, recebido numa conexão só para isso.
fn assinar(socket: &std::path::Path) -> Result<Arc<Mutex<Option<Value>>>, String> {
    let mut c = Cliente::conectar(socket).map_err(|e| e.to_string())?;
    c.chamar("robot.subscribe", json!({"hz": 50}))?;
    let ultimo = Arc::new(Mutex::new(None));
    let u = ultimo.clone();
    std::thread::spawn(move || {
        loop {
            let mut l = String::new();
            match c.leitor.read_line(&mut l) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            if let Ok(v) = serde_json::from_str::<Value>(&l)
                && v["method"] == "robot.state"
                && let Ok(mut g) = u.lock()
            {
                *g = Some(v["params"].clone());
            }
        }
    });
    Ok(ultimo)
}

struct Roteiro {
    c: Cliente,
    estado: Arc<Mutex<Option<Value>>>,
}

impl Roteiro {
    fn estado(&self) -> Value {
        self.estado
            .lock()
            .map(|g| g.clone().unwrap_or(Value::Null))
            .unwrap_or(Value::Null)
    }

    /// Mantém o pedido a 10 Hz, como o console, e espera a última resposta do laço.
    fn segurar(&mut self, twist: [f64; 3], por: Duration) -> Result<Value, String> {
        let fim = Instant::now() + por;
        while Instant::now() < fim {
            self.c.notificar(
                "robot.move",
                json!({"vx": twist[0], "vy": twist[1], "vyaw": twist[2]}),
            )?;
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(self.estado())
    }

    fn passo(&self, nome: &str, ok: bool, detalhe: &Value) -> Result<(), String> {
        println!("{} {nome}", if ok { "PASSOU" } else { "REPROVOU" });
        if ok {
            Ok(())
        } else {
            Err(format!("{nome}: {detalhe}"))
        }
    }

    fn checa_aplicado(&self, nome: &str, e: &Value, esperado: [f64; 3]) -> Result<(), String> {
        let a = &e["move"]["applied"];
        let ok = (0..3).all(|i| (a[i].as_f64().unwrap_or(f64::NAN) - esperado[i]).abs() < 1e-6);
        self.passo(nome, ok, e)
    }

    fn do_skill(&mut self, nome: &str) -> Result<(), String> {
        let r = self.c.chamar("robot.do", json!({"skill": nome}))?;
        self.passo(&format!("robot.do {nome}"), r["accepted"] == true, &r)?;
        std::thread::sleep(Duration::from_millis(60));
        let e = self.estado();
        self.passo(&format!("policy é {nome}"), e["policy"] == nome, &e)
    }

    fn completo(&mut self, v: f64) -> Result<(), String> {
        let saude = self.c.chamar("robot.health", json!({}))?;
        self.passo("robot.health saudável", saude["healthy"] == true, &saude)?;

        let e = self.segurar([v, 0.0, 0.0], Duration::from_millis(300))?;
        self.checa_aplicado("desarmado não anda", &e, [0.0; 3])?;
        self.passo(
            "desarmado diz por quê",
            e["move"]["limited_by"] == json!(["disarmed"]),
            &e,
        )?;
        self.passo("desarmado é held", e["policy"] == "held", &e)?;

        let r = self.c.chamar("robot.enable", json!({"on": true}))?;
        self.passo("robot.enable arma", r["accepted"] == true, &r)?;
        std::thread::sleep(Duration::from_millis(100));
        self.do_skill("modo_livre")?;

        for (nome, t) in [
            ("para a frente", [v, 0.0, 0.0]),
            ("para a esquerda", [0.0, v, 0.0]),
            ("girando à esquerda", [0.0, 0.0, 1.0]),
        ] {
            let e = self.segurar(t, Duration::from_millis(1000))?;
            self.checa_aplicado(nome, &e, t)?;
        }

        // Sem comando além do prazo, as rodas zeram sozinhas.
        std::thread::sleep(Duration::from_millis(800));
        let e = self.estado();
        self.checa_aplicado("sem comando, parado", &e, [0.0; 3])?;
        self.passo(
            "sem comando, diz deadman",
            e["move"]["limited_by"] == json!(["deadman"]),
            &e,
        )?;

        let r = self.c.chamar(
            "robot.head",
            json!({"neck_pitch": 0.0, "head_pitch": -0.2, "head_yaw": 0.3, "head_roll": 0.0}),
        )?;
        self.passo("robot.head", r["accepted"] == true, &r)?;
        std::thread::sleep(Duration::from_millis(600));

        let r = self.c.chamar(
            "robot.look",
            json!({"x": 1.0, "y": 0.0, "z": 0.2, "neck_pitch": 0.0}),
        )?;
        self.passo("robot.look responde clamped", r["clamped"] == false, &r)?;
        std::thread::sleep(Duration::from_millis(600));
        let r = self.c.chamar(
            "robot.look",
            json!({"x": 1.0, "y": 0.0, "z": 5.0, "neck_pitch": 0.0}),
        )?;
        self.passo(
            "robot.look fora de alcance é limitado",
            r["clamped"] == true,
            &r,
        )?;
        std::thread::sleep(Duration::from_millis(300));
        self.c.chamar(
            "robot.head",
            json!({"neck_pitch": 0.0, "head_pitch": 0.0, "head_yaw": 0.0, "head_roll": 0.0}),
        )?;
        std::thread::sleep(Duration::from_millis(600));

        self.do_skill("torre_segue_chassi")?;
        let t = [0.0, 0.0, 1.0];
        let e = self.segurar(t, Duration::from_millis(1000))?;
        self.checa_aplicado("torre segue: chassi gira", &e, t)?;

        self.do_skill("chassi_segue_torre")?;
        let t = [v, 0.0, 0.0];
        let e = self.segurar(t, Duration::from_millis(1000))?;
        self.checa_aplicado("chassi segue: anda", &e, t)?;
        let t = [0.0, 0.0, 1.0];
        let e = self.segurar(t, Duration::from_millis(1000))?;
        self.checa_aplicado("chassi segue: a torre gira e ele vai atrás", &e, t)?;
        self.segurar([0.0; 3], Duration::from_millis(300))?;

        self.do_skill("modo_livre")?;
        self.segurar([0.0; 3], Duration::from_millis(200))?;
        let r = self.c.chamar("robot.relax", json!({}))?;
        self.passo("robot.relax desarma", r["accepted"] == true, &r)?;
        std::thread::sleep(Duration::from_millis(100));
        let e = self.estado();
        self.passo("desarmado de novo", e["policy"] == "held", &e)?;
        Ok(())
    }

    fn so_andar(&mut self, v: f64) -> Result<(), String> {
        let r = self.c.chamar("robot.enable", json!({"on": true}))?;
        self.passo("robot.enable arma", r["accepted"] == true, &r)?;
        std::thread::sleep(Duration::from_millis(100));
        let t = [v, 0.0, 0.0];
        let e = self.segurar(t, Duration::from_millis(1000))?;
        self.checa_aplicado("anda", &e, t)?;
        self.c.chamar("robot.stop", json!({}))?;
        std::thread::sleep(Duration::from_millis(500));
        self.checa_aplicado("para", &self.estado(), [0.0; 3])?;
        self.c.chamar("robot.relax", json!({}))?;
        Ok(())
    }
}

fn principal(args: &Args) -> Result<(), String> {
    if let [a, b] = args.comparar.as_slice() {
        let d = rms1_control::gravacao::diferencas(a, b).map_err(|e| e.to_string())?;
        if d.is_empty() {
            println!("IGUAIS");
            return Ok(());
        }
        return Err(format!("quadros diferentes: {d:?}"));
    }
    let estado = assinar(&args.socket)?;
    let c =
        Cliente::conectar(&args.socket).map_err(|e| format!("{}: {e}", args.socket.display()))?;
    let mut r = Roteiro { c, estado };
    std::thread::sleep(Duration::from_millis(100));
    if args.so_andar {
        r.so_andar(args.velocidade)
    } else {
        r.completo(args.velocidade)
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    match principal(&args) {
        Ok(()) => {
            println!("ROTEIRO COMPLETO");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("ROTEIRO REPROVOU: {e}");
            ExitCode::FAILURE
        }
    }
}
