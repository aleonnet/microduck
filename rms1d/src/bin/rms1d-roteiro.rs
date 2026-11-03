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
    /// Não move nada: imprime toda a telemetria, uma vez por segundo, até Ctrl-C.
    #[arg(long)]
    ver: bool,
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

type Compartilhado = Arc<Mutex<Option<Value>>>;

/// O último `robot.state` e o último `s1.state`, recebidos numa conexão só para isso.
fn assinar(socket: &std::path::Path) -> Result<(Compartilhado, Compartilhado), String> {
    let mut c = Cliente::conectar(socket).map_err(|e| e.to_string())?;
    c.chamar("robot.subscribe", json!({"hz": 50}))?;
    c.chamar("s1.subscribe", json!({"hz": 50}))?;
    let ultimo = Arc::new(Mutex::new(None));
    let corpo = Arc::new(Mutex::new(None));
    let u = ultimo.clone();
    let k = corpo.clone();
    std::thread::spawn(move || {
        loop {
            let mut l = String::new();
            match c.leitor.read_line(&mut l) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let Ok(v) = serde_json::from_str::<Value>(&l) else {
                continue;
            };
            let destino = match v["method"].as_str() {
                Some("robot.state") => &u,
                Some("s1.state") => &k,
                _ => continue,
            };
            if let Ok(mut g) = destino.lock() {
                *g = Some(v["params"].clone());
            }
        }
    });
    Ok((ultimo, corpo))
}

/// O que imprime `--ver`: tudo, em português, numa linha por assunto.
fn ver(socket: &std::path::Path) -> Result<(), String> {
    let (_estado, corpo) = assinar(socket)?;
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let s = corpo
            .lock()
            .map(|g| g.clone().unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        println!("enlace   {}", s["link"]);
        println!("corpo    {}", s["body"]);
        println!("torre    {}", s["gimbal"]);
        println!("chassi   {}", s["chassis"]);
        println!("placas   {}", s["plates"]);
        println!("infrav.  {}", s["ir"]);
        println!();
    }
}

struct Roteiro {
    c: Cliente,
    estado: Compartilhado,
    corpo: Compartilhado,
}

/// Com pacote recente a torre no estado é a medida, e não chega exata ao alvo: 2 graus, em radianos.
const TOLERANCIA_TORRE_MEDIDA: f64 = 0.035;
const TOLERANCIA_TORRE_ALVO: f64 = 0.002;
/// Quanto esperar a torre chegar, com pacote recente: `velocidade_torre`, 120°/s, mais a folga do controlador.
const ESPERA_TORRE_MS: u64 = 1000;

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

    /// O que o laço diz que aplicou, e que o corpo aceitou os quadros. Isto não vê o robô andar: quem vê é o dono,
    /// no robô, e `test-rms1d-sim.sh`, no simulador. A telemetria entra na etapa 3.
    fn checa_aplicado(&mut self, nome: &str, e: &Value, esperado: [f64; 3]) -> Result<(), String> {
        let a = &e["move"]["applied"];
        let ok = (0..3).all(|i| (a[i].as_f64().unwrap_or(f64::NAN) - esperado[i]).abs() < 1e-6);
        self.passo(nome, ok, e)?;
        let saude = self.c.chamar("robot.health", json!({}))?;
        self.passo(
            &format!("{nome}: o corpo aceitou"),
            saude["bus"]["consecutive_errors"] == 0,
            &saude,
        )
    }

    fn corpo(&self) -> Value {
        self.corpo
            .lock()
            .map(|g| g.clone().unwrap_or(Value::Null))
            .unwrap_or(Value::Null)
    }

    /// Há telemetria neste corpo, e ela está recente.
    fn tem_telemetria(&self) -> bool {
        self.corpo()["link"]["fresh"] == true
    }

    /// Com o robô no ar a roda gira, e a medida diz o sentido: é a prova da ordem das rodas, sem o chão. Um pedido dado, rpm
    /// esperado em cada uma, com sinal de andar para a frente.
    fn checa_rodas(&self, nome: &str, esperado: [f64; 4]) -> Result<(), String> {
        if !self.tem_telemetria() {
            return Ok(());
        }
        let c = self.corpo();
        let rpm = &c["body"]["wheels_rpm"];
        let ok = (0..4).all(|i| {
            let m = rpm[i].as_f64().unwrap_or(f64::NAN);
            esperado[i].abs() < 1.0
                || (m.signum() == esperado[i].signum() && m.abs() > esperado[i].abs() * 0.5)
        });
        self.passo(
            &format!("{nome}: cada roda gira no sentido pedido"),
            ok,
            &json!({"medido": rpm, "esperado": esperado}),
        )
    }

    /// A torre tem de estar no alvo, em radianos do contrato: o publicado, ou o medido, se há telemetria.
    fn checa_torre(&self, nome: &str, yaw: f64, pitch: f64) -> Result<(), String> {
        let medida = self.tem_telemetria();
        if medida {
            std::thread::sleep(Duration::from_millis(ESPERA_TORRE_MS));
        }
        let tol = if medida {
            TOLERANCIA_TORRE_MEDIDA
        } else {
            TOLERANCIA_TORRE_ALVO
        };
        let e = self.estado();
        let h = &e["head"];
        let perto = |v: &Value, alvo: f64| (v.as_f64().unwrap_or(f64::NAN) - alvo).abs() < tol;
        self.passo(nome, perto(&h[1], pitch) && perto(&h[2], yaw), &e)
    }

    fn do_skill(&mut self, nome: &str) -> Result<(), String> {
        let r = self.c.chamar("robot.do", json!({"skill": nome}))?;
        self.passo(&format!("robot.do {nome}"), r["accepted"] == true, &r)?;
        std::thread::sleep(Duration::from_millis(60));
        let e = self.estado();
        self.passo(&format!("policy é {nome}"), e["policy"] == nome, &e)
    }

    /// Toda a telemetria que este corpo tem chegou? O corpo falso diz que não tem, e aí não há o que conferir.
    fn checa_telemetria(&mut self) -> Result<(), String> {
        let inicio = Instant::now();
        loop {
            let c = self.corpo();
            let s = c["link"]["state"].as_str().unwrap_or("").to_owned();
            if s == "off" {
                println!("SEM TELEMETRIA neste corpo");
                return Ok(());
            }
            if s == "receiving" && c["link"]["fresh"] == true {
                let b = &c["body"];
                let tudo = [
                    "velocity_ground",
                    "velocity_body",
                    "wheels_rpm",
                    "position_m",
                    "acc_g",
                    "gyro_rad_s",
                    "attitude_deg",
                    "battery",
                ]
                .iter()
                .all(|k| !b[*k].is_null())
                    && !c["gimbal"].is_null();
                return self.passo("a telemetria chega, com todos os dados", tudo, &c);
            }
            if s == "refused" || inicio.elapsed() > Duration::from_secs(8) {
                return self.passo("a telemetria chega", false, &c);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn completo(&mut self, v: f64) -> Result<(), String> {
        let saude = self.c.chamar("robot.health", json!({}))?;
        self.passo("robot.health saudável", saude["healthy"] == true, &saude)?;
        self.checa_telemetria()?;

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
            let geometria = rms1d_params::Params::default().geometria;
            let esperado = rms1_control::cinematica::rpm_de_twist(
                t,
                geometria.raio_roda_m,
                geometria.meia_soma_m,
            );
            self.checa_rodas(nome, esperado)?;
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
        self.checa_torre("robot.head chega ao alvo", 0.3, -0.2)?;

        let r = self.c.chamar(
            "robot.look",
            json!({"x": 1.0, "y": 0.0, "z": 0.2, "neck_pitch": 0.0}),
        )?;
        self.passo("robot.look responde clamped", r["clamped"] == false, &r)?;
        std::thread::sleep(Duration::from_millis(600));
        let pitch = r["head"]["head_pitch"].as_f64().unwrap_or(f64::NAN);
        self.checa_torre("robot.look chega ao alvo", 0.0, pitch)?;
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
        let pitch = r["head"]["head_pitch"].as_f64().unwrap_or(f64::NAN);
        self.checa_torre("o limite é o que aparece no estado", 0.0, pitch)?;
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
        let t = [0.0, v, 0.0];
        let e = self.segurar(t, Duration::from_millis(1000))?;
        self.checa_aplicado("chassi segue: anda para a esquerda", &e, t)?;
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
        let saude = self.c.chamar("robot.health", json!({}))?;
        self.passo("segue saudável no fim", saude["healthy"] == true, &saude)?;
        Ok(())
    }

    fn so_andar(&mut self, v: f64) -> Result<(), String> {
        let r = self.c.chamar("robot.enable", json!({"on": true}))?;
        self.passo("robot.enable arma", r["accepted"] == true, &r)?;
        std::thread::sleep(Duration::from_millis(100));
        // O modo fica de um cliente para o outro, e andar reto é com as rodas.
        self.do_skill("modo_livre")?;
        let t = [v, 0.0, 0.0];
        let e = self.segurar(t, Duration::from_millis(1000))?;
        self.checa_aplicado("anda", &e, t)?;
        self.c.chamar("robot.stop", json!({}))?;
        std::thread::sleep(Duration::from_millis(500));
        let e = self.estado();
        self.checa_aplicado("para", &e, [0.0; 3])?;
        self.c.chamar("robot.relax", json!({}))?;
        Ok(())
    }
}

fn comparar(a: &std::path::Path, b: &std::path::Path) -> ExitCode {
    match rms1_control::gravacao::diferencas(a, b) {
        Ok(d) if d.is_empty() => {
            println!("IGUAIS");
            ExitCode::SUCCESS
        }
        Ok(d) => {
            eprintln!("QUADROS DIFERENTES: {d:?}");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("NÃO DÁ PARA COMPARAR: {e}");
            ExitCode::FAILURE
        }
    }
}

fn principal(args: &Args) -> Result<(), String> {
    if args.ver {
        return ver(&args.socket);
    }
    let (estado, corpo) = assinar(&args.socket)?;
    let c =
        Cliente::conectar(&args.socket).map_err(|e| format!("{}: {e}", args.socket.display()))?;
    let mut r = Roteiro { c, estado, corpo };
    std::thread::sleep(Duration::from_millis(100));
    if args.so_andar {
        r.so_andar(args.velocidade)
    } else {
        r.completo(args.velocidade)
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    if let [a, b] = args.comparar.as_slice() {
        return comparar(a, b);
    }
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
