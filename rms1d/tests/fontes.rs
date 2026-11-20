//! Quem manda no corpo, pelo socket, com o processo de verdade e três clientes: o piloto, o agente e um que não diz o
//! nome. A tabela é a de produção (a padrão de `rms1d-params`, a mesma de `deploy/s1/rms1d.toml`).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const RMS1D: &str = env!("CARGO_BIN_EXE_rms1d");
const FALSO: &str = "[corpo]\ntipo = \"falso\"\n";
const PRAZO: Duration = Duration::from_millis(500);

/// Curto de propósito: caminho de socket tem limite de tamanho.
fn pasta(nome: &str) -> PathBuf {
    let d = PathBuf::from(format!("/tmp/rms1-f{}-{nome}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Processo {
    filho: Child,
    socket: PathBuf,
    pasta: PathBuf,
}

impl Drop for Processo {
    fn drop(&mut self) {
        let _ = self.filho.kill();
        let _ = self.filho.wait();
        let _ = std::fs::remove_dir_all(&self.pasta);
    }
}

fn subir(nome: &str) -> Processo {
    let d = pasta(nome);
    let cfg = d.join("rms1d.toml");
    std::fs::write(&cfg, FALSO).unwrap();
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
    Processo {
        filho,
        socket,
        pasta: d,
    }
}

struct Cliente(UnixStream, BufReader<UnixStream>, u64);

impl Cliente {
    /// Liga e, com nome, diz quem é no `hello`.
    fn novo(s: &Path, nome: Option<&str>) -> Self {
        let f = UnixStream::connect(s).unwrap();
        f.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let l = BufReader::new(f.try_clone().unwrap());
        let mut c = Self(f, l, 1);
        if let Some(n) = nome {
            let r = c.chamar("hello", json!({"api_version": 68, "client": n}));
            assert!(r["error"].is_null(), "{n}: {r}");
        }
        c
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

    /// Sem id: é assim que os pedidos contínuos chegam.
    fn notificar(&mut self, m: &str, p: Value) {
        writeln!(self.0, "{}", json!({"jsonrpc":"2.0","method":m,"params":p})).unwrap();
    }

    fn andar(&mut self, twist: [f64; 3]) {
        self.notificar(
            "robot.move",
            json!({"vx": twist[0], "vy": twist[1], "vyaw": twist[2]}),
        );
    }

    fn girar_torre(&mut self, yaw: f64) {
        self.notificar(
            "s1.gimbal",
            json!({"action": "speed", "yaw": yaw, "pitch": 0.0}),
        );
    }

    /// A próxima notificação deste método.
    fn proxima(&mut self, metodo: &str) -> Value {
        loop {
            let mut l = String::new();
            self.1.read_line(&mut l).unwrap();
            let v: Value = serde_json::from_str(&l).unwrap();
            if v["method"] == metodo {
                return v["params"].clone();
            }
        }
    }
}

/// Quem olha: assina `robot.state` e `s1.state` no ritmo do laço, e não pede nada.
struct Olho(Cliente);

impl Olho {
    fn novo(s: &Path) -> Self {
        let mut c = Cliente::novo(s, None);
        c.chamar("robot.subscribe", json!({}));
        c.chamar("s1.subscribe", json!({}));
        Self(c)
    }

    fn robo(&mut self) -> Value {
        self.0.proxima("robot.state")
    }

    fn s1(&mut self) -> Value {
        self.0.proxima("s1.state")
    }

    /// Lê `robot.state` até valer, e devolve quanto levou. Passado o limite, falha com o último estado.
    fn ate(&mut self, limite: Duration, o_que: &str, vale: impl Fn(&Value) -> bool) -> Duration {
        let t = Instant::now();
        loop {
            let e = self.robo();
            if vale(&e) {
                return t.elapsed();
            }
            assert!(t.elapsed() < limite, "{o_que}: {}", e["move"]);
        }
    }

    /// O mesmo, pelo bloco `control` de `s1.state`.
    fn ate_controle(
        &mut self,
        limite: Duration,
        o_que: &str,
        vale: impl Fn(&Value) -> bool,
    ) -> Duration {
        let t = Instant::now();
        loop {
            let c = self.s1()["control"].clone();
            if vale(&c) {
                return t.elapsed();
            }
            assert!(t.elapsed() < limite, "{o_que}: {c}");
        }
    }
}

fn aplicado(e: &Value) -> [f64; 3] {
    let a = &e["move"]["applied"];
    [0, 1, 2].map(|i| a[i].as_f64().unwrap())
}

const FRENTE: [f64; 3] = [0.3, 0.0, 0.0];
const GIRO: [f64; 3] = [0.0, 0.0, 0.5];
const LOGO: Duration = Duration::from_millis(300);

/// O piloto arma: o agente não pode.
fn armar(piloto: &mut Cliente) {
    let r = piloto.chamar("robot.enable", json!({"on": true}));
    assert_eq!(r["result"]["accepted"], true, "{r}");
}

/// Aceite 4: o agente anda e o piloto pede movimento. O do piloto é aplicado em seguida, e `move.source` diz quem pediu.
#[test]
fn o_piloto_toma_as_rodas_do_agente_na_hora() {
    let p = subir("toma");
    let mut olho = Olho::novo(&p.socket);
    let mut piloto = Cliente::novo(&p.socket, Some("piloto"));
    let mut agente = Cliente::novo(&p.socket, Some("agente"));
    armar(&mut piloto);
    agente.andar(FRENTE);
    olho.ate(LOGO, "o agente não andou", |e| {
        aplicado(e) == FRENTE && e["move"]["source"] == "agente"
    });
    let c = olho.s1()["control"].clone();
    assert_eq!(c["wheels"]["source"], "agente", "{c}");
    assert_eq!(c["gimbal"]["source"], Value::Null, "{c}");
    // O piloto pede, e o agente segue renovando: o último a escrever é o agente, e quem anda é o piloto.
    piloto.andar(GIRO);
    agente.andar(FRENTE);
    let levou = olho.ate(LOGO, "o aplicado segue o do agente", |e| {
        aplicado(e) == GIRO
    });
    assert!(levou < Duration::from_millis(100), "na hora: {levou:?}");
    let e = olho.robo();
    assert_eq!(
        e["move"]["source"], "piloto",
        "a fonte publicada é outra: {e}"
    );
    assert_eq!(e["move"]["requested"], json!(GIRO));
    assert_eq!(olho.s1()["control"]["wheels"]["source"], "piloto");
    // E diz a cada conexão qual linha da tabela ela é: quem olha não disse o nome.
    assert_eq!(c["client"], "sem_nome", "{c}");
    // `s1.telemetry`, que responde sem esperar o laço, diz o mesmo.
    let t = agente.chamar("s1.telemetry", json!({}));
    assert_eq!(t["result"]["control"]["client"], "agente", "{t}");
    assert_eq!(t["result"]["control"]["wheels"]["source"], "piloto", "{t}");
}

/// O piloto solta e o agente segue renovando: as rodas voltam ao agente depois do prazo do piloto, e não antes.
#[test]
fn solto_o_piloto_o_agente_retoma_depois_do_prazo() {
    let p = subir("solta");
    let mut olho = Olho::novo(&p.socket);
    let mut piloto = Cliente::novo(&p.socket, Some("piloto"));
    let mut agente = Cliente::novo(&p.socket, Some("agente"));
    armar(&mut piloto);
    piloto.andar(GIRO);
    let soltou = Instant::now();
    let mut voltou = None;
    // Sem dormir: o estado chega a cada ciclo do laço, e quem dorme lê estado velho. O agente renova a cada cinco.
    let mut lidos = 0;
    while soltou.elapsed() < PRAZO + LOGO {
        if lidos % 5 == 0 {
            agente.andar(FRENTE);
        }
        lidos += 1;
        let e = olho.robo();
        if aplicado(&e) == FRENTE {
            voltou = Some(soltou.elapsed());
            assert_eq!(e["move"]["source"], "agente", "{e}");
            break;
        }
        assert_eq!(aplicado(&e), GIRO, "nem um ciclo a zero na troca: {e}");
    }
    let voltou = voltou.expect("as rodas não voltaram ao agente");
    assert!(
        voltou >= PRAZO - Duration::from_millis(40),
        "voltaram antes do prazo do piloto: {voltou:?}"
    );
}

/// Aceite 6: a parada vale de qualquer fonte, inclusive da de menor prioridade, com o piloto dono das rodas e da torre.
#[test]
fn a_parada_da_fonte_menor_zera_as_rodas_e_a_torre() {
    let p = subir("para");
    let mut olho = Olho::novo(&p.socket);
    let mut piloto = Cliente::novo(&p.socket, Some("piloto"));
    let mut qualquer = Cliente::novo(&p.socket, None);
    armar(&mut piloto);
    piloto.andar(GIRO);
    piloto.girar_torre(30.0);
    olho.ate(LOGO, "o piloto não andou", |e| aplicado(e) == GIRO);
    olho.ate_controle(LOGO, "o piloto não girou a torre", |c| {
        c["gimbal"]["source"] == "piloto"
    });
    let r = qualquer.chamar("robot.stop", json!({}));
    assert_eq!(r["result"]["accepted"], true, "{r}");
    // Bem antes do prazo do pedido do piloto: quem zera é a parada.
    let levou = olho.ate(LOGO, "o aplicado não vai a zero", |e| {
        aplicado(e) == [0.0; 3]
    });
    assert!(levou < Duration::from_millis(150), "{levou:?}");
    let e = olho.robo();
    assert_eq!(
        e["move"]["limited_by"],
        Value::Null,
        "parar não é prazo vencido: {e}"
    );
    let c = olho.s1()["control"].clone();
    assert_eq!(
        c["wheels"]["source"], "sem_nome",
        "o zero de quem parou: {c}"
    );
    assert_eq!(
        c["gimbal"]["source"], "sem_nome",
        "a velocidade da torre do piloto ficou: {c}"
    );
    // Quem tem prioridade volta a mandar no pedido seguinte.
    piloto.andar(GIRO);
    olho.ate(LOGO, "o piloto não retomou", |e| aplicado(e) == GIRO);
}

/// Aceite 7: uma fonte pede num canal que não é dela. É recusada com o motivo, e o corpo não muda.
#[test]
fn o_agente_nao_arma_nem_muda_modo_ajuste_ou_autonomia() {
    let p = subir("canal");
    let mut olho = Olho::novo(&p.socket);
    let mut agente = Cliente::novo(&p.socket, Some("agente"));
    for (m, params, canal) in [
        ("robot.enable", json!({"on": true}), "energia"),
        (
            "robot.enable",
            json!({"on": false, "toggle": true}),
            "energia",
        ),
        ("robot.init", json!({}), "energia"),
        ("robot.do", json!({"skill": "torre_segue_chassi"}), "modo"),
        ("robot.autonomous", json!({"active": true}), "autonomia"),
    ] {
        let r = agente.chamar(m, params);
        assert_eq!(r["result"]["accepted"], false, "{m}: {r}");
        let por = r["result"]["reason"].as_str().unwrap();
        assert!(por.contains("agente") && por.contains(canal), "{m}: {por}");
    }
    // Como notificação, é ignorada.
    agente.notificar("robot.do", json!({"skill": "torre_segue_chassi"}));
    // Gravar um ajuste responde documento: quem não pode recebe erro. Ler, pode.
    for (m, params) in [
        ("s1.armor", json!({"sensitivity": 3})),
        ("s1.sight", json!({"zoom_in": [1, 1], "zoom_out": [1, 1]})),
    ] {
        let r = agente.chamar(m, params);
        assert_eq!(r["error"]["code"], 14, "{m}: {r}");
        assert!(
            r["error"]["message"].as_str().unwrap().contains("ajustes"),
            "{r}"
        );
        let r = agente.chamar(m, json!({}));
        assert!(r["error"].is_null(), "ler {m}: {r}");
    }
    let e = olho.robo();
    assert_eq!(e["policy"], "held", "o robô arma: {e}");
    assert_eq!(e["autonomous"], Value::Null, "{e}");
    // O que é dele, ele pode: as rodas, a torre, e parar.
    let mut piloto = Cliente::novo(&p.socket, Some("piloto"));
    armar(&mut piloto);
    let r = agente.chamar("robot.move", json!({"vx": 0.3, "vy": 0.0, "vyaw": 0.0}));
    assert_eq!(r["result"]["accepted"], true, "{r}");
    let r = agente.chamar("s1.gimbal", json!({"action": "stop"}));
    assert_eq!(r["result"]["accepted"], true, "{r}");
    let r = agente.chamar("robot.stop", json!({}));
    assert_eq!(r["result"]["accepted"], true, "{r}");
    assert_eq!(olho.robo()["policy"], "modo_livre", "e o modo não mudou");
}

/// Aceite 8: o cérebro pede com a autonomia desligada, e nada acontece. Ligada por quem pode, o pedido vale, e o estado
/// publica `autonomous`. Desligada, ele para na hora.
#[test]
fn o_cerebro_so_anda_com_a_autonomia_ligada() {
    let p = subir("auto");
    let mut olho = Olho::novo(&p.socket);
    let mut piloto = Cliente::novo(&p.socket, Some("piloto"));
    let mut cerebro = Cliente::novo(&p.socket, Some("cerebro"));
    armar(&mut piloto);
    cerebro.andar(FRENTE);
    let r = cerebro.chamar("robot.move", json!({"vx": 0.3, "vy": 0.0, "vyaw": 0.0}));
    assert_eq!(r["result"]["accepted"], false, "{r}");
    assert!(
        r["result"]["reason"]
            .as_str()
            .unwrap()
            .contains("autonomia ligada"),
        "{r}"
    );
    for _ in 0..10 {
        let e = olho.robo();
        assert_eq!(
            aplicado(&e),
            [0.0; 3],
            "o pedido vale com a autonomia desligada"
        );
        assert_eq!(e["autonomous"], Value::Null, "{e}");
    }
    assert_eq!(
        piloto.chamar("robot.mode", json!({}))["result"]["autonomous"],
        Value::Null
    );
    // O cérebro não se liga sozinho.
    let r = cerebro.chamar("robot.autonomous", json!({"active": true}));
    assert_eq!(r["result"]["accepted"], false, "{r}");
    let r = piloto.chamar("robot.autonomous", json!({"active": true}));
    assert_eq!(r["result"]["accepted"], true, "{r}");
    assert_eq!(
        piloto.chamar("robot.mode", json!({}))["result"]["autonomous"],
        true
    );
    cerebro.andar(FRENTE);
    olho.ate(LOGO, "o cérebro não andou com a autonomia ligada", |e| {
        aplicado(e) == FRENTE && e["autonomous"] == true && e["move"]["source"] == "cerebro"
    });
    let c = olho.s1()["control"].clone();
    assert_eq!(
        (&c["autonomous"], &c["wheels"]["source"]),
        (&json!(true), &json!("cerebro"))
    );
    // Desligada, o que ele pediu deixa de valer sem esperar o prazo.
    piloto.chamar("robot.autonomous", json!({"active": false}));
    let levou = olho.ate(LOGO, "o cérebro seguiu andando", |e| {
        aplicado(e) == [0.0; 3]
    });
    assert!(levou < Duration::from_millis(150), "{levou:?}");
}

/// Aceite 10: o agente gira a torre e o piloto pede velocidade da torre. Vale a do piloto, e `control.gimbal.source` o
/// diz; parado o piloto, a torre volta ao agente depois do prazo.
#[test]
fn na_torre_o_piloto_tambem_toma_e_o_prazo_devolve() {
    let p = subir("torre");
    let mut olho = Olho::novo(&p.socket);
    let mut piloto = Cliente::novo(&p.socket, Some("piloto"));
    let mut agente = Cliente::novo(&p.socket, Some("agente"));
    armar(&mut piloto);
    agente.girar_torre(20.0);
    olho.ate_controle(LOGO, "o agente não tem a torre", |c| {
        c["gimbal"]["source"] == "agente"
    });
    piloto.girar_torre(-30.0);
    let soltou = Instant::now();
    agente.girar_torre(20.0);
    let levou = olho.ate_controle(LOGO, "a torre segue a do agente", |c| {
        c["gimbal"]["source"] == "piloto"
    });
    assert!(levou < Duration::from_millis(100), "na hora: {levou:?}");
    // Como requisição, a do agente é recusada dizendo quem tem a torre; e ele não troca o alvo de ângulo.
    let r = agente.chamar(
        "s1.gimbal",
        json!({"action": "speed", "yaw": 20.0, "pitch": 0.0}),
    );
    assert_eq!(r["result"]["accepted"], false, "{r}");
    assert_eq!(r["result"]["reason"], "piloto tem a torre agora", "{r}");
    let r = agente.chamar("robot.look", json!({"x": 1.0, "y": 0.0, "z": 0.0}));
    assert_eq!(r["error"]["code"], 1, "ocupado: {r}");
    let mut voltou = None;
    let mut lidos = 0;
    while soltou.elapsed() < PRAZO + LOGO {
        if lidos % 5 == 0 {
            agente.girar_torre(20.0);
        }
        lidos += 1;
        if olho.s1()["control"]["gimbal"]["source"] == "agente" {
            voltou = Some(soltou.elapsed());
            break;
        }
    }
    let voltou = voltou.expect("a torre não voltou ao agente");
    assert!(
        voltou >= PRAZO - Duration::from_millis(40),
        "voltou antes do prazo do piloto: {voltou:?}"
    );
}

/// Aceite 13, do lado do corpo: quando o agente larga o canal dele (rodas a zero, torre parada) com o piloto pedindo, o
/// pedido do piloto segue aplicado sem um ciclo a zero. Com `robot.stop` haveria: é por isso que o agente deixou de usá-lo
/// para arrumar a casa.
#[test]
fn o_agente_soltar_o_canal_nao_para_o_piloto() {
    let p = subir("larga");
    let mut olho = Olho::novo(&p.socket);
    let mut piloto = Cliente::novo(&p.socket, Some("piloto"));
    let mut agente = Cliente::novo(&p.socket, Some("agente"));
    armar(&mut piloto);
    agente.andar(FRENTE);
    piloto.andar(GIRO);
    olho.ate(LOGO, "o piloto não tomou", |e| aplicado(e) == GIRO);
    // O fim de uma habilidade do agente, como ele o faz agora.
    agente.andar([0.0; 3]);
    agente.notificar("s1.gimbal", json!({"action": "stop"}));
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(250) {
        piloto.andar(GIRO);
        let e = olho.robo();
        assert_eq!(aplicado(&e), GIRO, "o aplicado vai a zero: {e}");
    }
    // A parada, essa sim, para o piloto por um instante: é a parada pedida por alguém.
    agente.chamar("robot.stop", json!({}));
    olho.ate(LOGO, "a parada do agente não parou", |e| {
        aplicado(e) == [0.0; 3]
    });
}

/// O pedido de rodas por requisição que perde para o piloto é recusado dizendo quem tem as rodas, e não fica guardado:
/// quando o piloto solta, o robô não sai andando por um pedido que foi recusado.
#[test]
fn a_requisicao_que_perde_e_recusada_e_nao_fica_guardada() {
    let p = subir("req");
    let mut olho = Olho::novo(&p.socket);
    let mut piloto = Cliente::novo(&p.socket, Some("piloto"));
    let mut agente = Cliente::novo(&p.socket, Some("agente"));
    armar(&mut piloto);
    piloto.andar(GIRO);
    olho.ate(LOGO, "o piloto não andou", |e| aplicado(e) == GIRO);
    std::thread::sleep(Duration::from_millis(200));
    let r = agente.chamar("robot.move", json!({"vx": 0.3, "vy": 0.0, "vyaw": 0.0}));
    assert_eq!(r["result"]["accepted"], false, "{r}");
    assert_eq!(r["result"]["reason"], "piloto tem as rodas agora", "{r}");
    // O do piloto vence aos 500 ms; o do agente, se guardado, valeria até os 700.
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(600) {
        let e = olho.robo();
        assert_ne!(aplicado(&e), FRENTE, "o pedido recusado foi aplicado: {e}");
    }
}

/// Aceite 9: quem não diz o nome segue servido como antes da tabela, e perde para todos. E o `hello`: o nome que a tabela
/// não tem fica como a fonte sem nome, o nome dela é recusado, e campo desconhecido também.
#[test]
fn quem_nao_diz_o_nome_segue_podendo_e_perde_para_todos() {
    let p = subir("semnome");
    let mut olho = Olho::novo(&p.socket);
    let mut qualquer = Cliente::novo(&p.socket, None);
    let r = qualquer.chamar("robot.enable", json!({"on": true}));
    assert_eq!(r["result"]["accepted"], true, "arma, como antes: {r}");
    qualquer.andar(FRENTE);
    olho.ate(LOGO, "quem não diz o nome não andou", |e| {
        aplicado(e) == FRENTE && e["move"]["source"].is_null()
    });
    assert_eq!(olho.s1()["control"]["wheels"]["source"], "sem_nome");
    let mut agente = Cliente::novo(&p.socket, Some("agente"));
    agente.andar(GIRO);
    qualquer.andar(FRENTE);
    olho.ate(LOGO, "o agente não venceu quem não diz o nome", |e| {
        aplicado(e) == GIRO
    });
    // Um nome fora da tabela (o `padd` do pato) é servido, como a fonte sem nome.
    let mut padd = Cliente::novo(&p.socket, Some("padd"));
    let r = padd.chamar("robot.do", json!({"skill": "modo_livre"}));
    assert_eq!(r["result"]["accepted"], true, "{r}");
    let r = padd.chamar("robot.move", json!({"vx": 0.3, "vy": 0.0, "vyaw": 0.0}));
    assert_eq!(r["result"]["reason"], "agente tem as rodas agora", "{r}");
    let t = padd.chamar("s1.telemetry", json!({}));
    assert_eq!(
        t["result"]["control"]["client"], "sem_nome",
        "o corpo diz ao cliente que o nome dele não está na tabela: {t}"
    );
    // O nome da fonte sem nome ninguém pode dizer.
    let r = padd.chamar("hello", json!({"api_version": 68, "client": "sem_nome"}));
    assert_eq!(r["error"]["code"], -32602, "{r}");
    // E o `hello` recusa campo desconhecido: a página manda exatamente `api_version` e `client`.
    let r = padd.chamar(
        "hello",
        json!({"api_version": 68, "client": "piloto", "extra": 1}),
    );
    assert!(!r["error"].is_null(), "{r}");
    // Um `hello` novo troca quem a conexão é.
    let r = padd.chamar("hello", json!({"api_version": 68, "client": "piloto"}));
    assert_eq!(r["result"]["api_version"], 68, "{r}");
    padd.andar(FRENTE);
    olho.ate(LOGO, "a conexão não virou o piloto", |e| {
        aplicado(e) == FRENTE && e["move"]["source"] == "piloto"
    });
}
