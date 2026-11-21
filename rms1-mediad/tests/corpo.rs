//! O cliente do `rms1d`: cada estado que ele publica vira uma amostra no mundo, a queda da ligação vira "sem a pose", e a
//! ligação volta sozinha. O `rms1d` daqui é de mentira: um socket numa pasta temporária, que o teste dirige.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rms1_mediad::corpo::{self, Relogio};
use rms1_mundo::{Corpo, Mundo, Olho};
use rms1d_params::{Fisica, Midia, MidiaMundo};

/// Um `s1.state` como o `rms1d` o publica (`rms1d/src/respostas.rs`, `estado_s1`), com o que este cliente lê.
fn estado(recente: bool, giro: f64, inclinacao_no_chao: f64, pose: [f64; 3]) -> String {
    let v = serde_json::json!({"jsonrpc": "2.0", "method": "s1.state", "params": {
        "link": {"state": "ok", "fresh": recente},
        "body": {"position_m": [pose[0], pose[1], 0.0], "attitude_deg": {"yaw": pose[2], "pitch": 0.0, "roll": 0.0}},
        // A inclinação em relação à torre vai diferente da do chão, para o teste ver qual das duas é lida.
        "gimbal": {"yaw_deg": giro, "pitch_deg": 77.0, "yaw_ground_deg": 99.0, "pitch_ground_deg": inclinacao_no_chao},
    }});
    format!("{v}\n")
}

#[test]
fn o_estado_do_rms1d_vira_amostra_em_radianos_com_a_torre_no_chao() {
    let v: serde_json::Value =
        serde_json::from_str(&estado(true, 90.0, -10.0, [1.5, -0.25, 180.0])).unwrap();
    let a = corpo::amostra_de(&v["params"], 7.5).expect("um estado inteiro dá amostra");
    assert_eq!(a.t, 7.5);
    let perto = |v: f64, esperado: f64| (v - esperado).abs() < 1e-12;
    assert!(
        perto(a.torre[0], std::f64::consts::FRAC_PI_2),
        "o giro da torre é o do chassi, em radianos: {a:?}"
    );
    assert!(
        perto(a.torre[1], (-10.0f64).to_radians()),
        "a inclinação é a do chão, e não a da torre no chassi: {a:?}"
    );
    assert!(
        perto(a.pose[0], 1.5) && perto(a.pose[1], -0.25) && perto(a.pose[2], std::f64::consts::PI),
        "a pose em metros e o rumo em radianos: {a:?}"
    );
    // Sem pacote recente do corpo, sem a torre ou sem a pose, não há amostra: número velho situaria o quadro errado.
    let velho: serde_json::Value =
        serde_json::from_str(&estado(false, 90.0, -10.0, [1.5, -0.25, 180.0])).unwrap();
    assert!(corpo::amostra_de(&velho["params"], 7.5).is_none());
    for falta in ["body", "gimbal"] {
        let mut sem = v["params"].clone();
        sem.as_object_mut().unwrap().remove(falta);
        assert!(
            corpo::amostra_de(&sem, 7.5).is_none(),
            "sem {falta} não há amostra"
        );
    }
    // O socket que o arquivo traz de fábrica é o do contrato, onde o `rms1d` serve no robô.
    assert_eq!(Midia::default().robo, duck_ipc_proto::socket::ROBOT);
}

/// Espera até `vale` dar verdadeiro, por no máximo `por`.
fn ate(por: Duration, mut vale: impl FnMut() -> bool) -> bool {
    let fim = Instant::now() + por;
    while Instant::now() < fim {
        if vale() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    vale()
}

/// O que o cliente manda ao ligar: tudo o que chega em 300 ms.
fn ao_ligar(conexao: &UnixStream) -> Vec<serde_json::Value> {
    conexao
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let mut leitor = BufReader::new(conexao.try_clone().unwrap());
    let mut linhas = Vec::new();
    let mut linha = String::new();
    while leitor.read_line(&mut linha).is_ok_and(|n| n > 0) {
        linhas.push(serde_json::from_str(&linha).unwrap());
        linha.clear();
    }
    linhas
}

#[test]
fn o_cliente_leva_as_amostras_ao_mundo_diz_quando_faltam_e_religa_sozinho() {
    let pasta = tempfile::tempdir().unwrap();
    let socket = pasta.path().join("robot.sock");
    let ouvinte = UnixListener::bind(&socket).unwrap();

    let f = Fisica::da_tabela();
    let olho = Olho {
        lente: f.camera_simulada,
        no_corpo: f.camera_no_corpo,
    };
    let corpo_do_robo = Corpo {
        largura_m: f.largura_m,
        frente_m: f.frente_m,
    };
    let ajustes = MidiaMundo::default();
    let vale = Duration::from_millis(ajustes.torre_vale_ms);
    let mundo = Arc::new(Mutex::new(
        Mundo::novo(Some(olho), corpo_do_robo, ajustes).unwrap(),
    ));
    let relogio = Relogio::default();
    let parar = Arc::new(AtomicBool::new(false));
    let amostras = Arc::new(Mutex::new(Vec::new()));
    let (m, vistas, p, caminho) = (
        mundo.clone(),
        amostras.clone(),
        parar.clone(),
        socket.clone(),
    );
    let cliente = std::thread::spawn(move || {
        corpo::rodar(
            &caminho,
            relogio,
            move |a| {
                vistas.lock().unwrap().push(a);
                m.lock().unwrap().amostra(a);
            },
            p,
        );
    });
    let percepcao =
        |m: &Arc<Mutex<Mundo>>| m.lock().unwrap().agora(relogio.agora()).percepcao.nome();

    // Antes de o `rms1d` dizer algo, o mundo não tem a pose.
    assert_eq!(percepcao(&mundo), "no_pose");
    let (mut conexao, _) = ouvinte.accept().unwrap();
    // O cliente só assina: não diz nome (seria uma fonte da tabela) nem manda mais nada.
    let pedidos = ao_ligar(&conexao);
    assert_eq!(pedidos.len(), 1, "{pedidos:?}");
    assert_eq!(
        (
            pedidos[0]["method"].as_str(),
            pedidos[0]["params"]["hz"].as_u64(),
            pedidos[0]["id"].is_null()
        ),
        (Some("s1.subscribe"), Some(50), false),
        "{pedidos:?}"
    );
    conexao
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"accepted\":true}}\n")
        .unwrap();
    // Um estado sem pacote recente do corpo não vira amostra; os seguintes viram, na ordem.
    conexao
        .write_all(estado(false, 5.0, 0.0, [9.0, 9.0, 9.0]).as_bytes())
        .unwrap();
    for giro in [10.0, 20.0, 30.0] {
        conexao
            .write_all(estado(true, giro, -5.0, [0.5, 0.25, 90.0]).as_bytes())
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        ate(Duration::from_secs(2), || amostras.lock().unwrap().len()
            == 3),
        "{:?}",
        amostras.lock().unwrap()
    );
    let vistas = amostras.lock().unwrap().clone();
    let giros: Vec<f64> = vistas
        .iter()
        .map(|a| a.torre[0].to_degrees().round())
        .collect();
    assert_eq!(giros, [10.0, 20.0, 30.0]);
    assert!(
        vistas.windows(2).all(|w| w[1].t > w[0].t) && vistas[2].t <= relogio.agora(),
        "cada amostra leva a hora em que chegou, no relógio do serviço: {vistas:?}"
    );
    // Com a pose em dia e sem grade nenhuma, o que falta é a grade, e não a pose.
    assert_eq!(percepcao(&mundo), "stale");

    // O `rms1d` cai: passado o prazo da torre, o mundo diz que não tem a pose.
    drop(conexao);
    std::thread::sleep(vale + Duration::from_millis(100));
    assert_eq!(percepcao(&mundo), "no_pose");

    // E o cliente religa sozinho, assina de novo, e as amostras voltam.
    let (mut conexao, _) = ouvinte.accept().unwrap();
    assert_eq!(ao_ligar(&conexao).len(), 1, "a assinatura de novo");
    conexao
        .write_all(estado(true, 40.0, -5.0, [0.5, 0.25, 90.0]).as_bytes())
        .unwrap();
    assert!(
        ate(Duration::from_secs(2), || amostras.lock().unwrap().len()
            == 4),
        "a amostra de depois de religar não chegou"
    );
    assert_eq!(percepcao(&mundo), "stale");

    // Uma linha sem fim não é do `rms1d`: o cliente não a guarda, larga a ligação e liga de novo.
    conexao.write_all(&vec![b'x'; 70 * 1024]).unwrap();
    ouvinte.set_nonblocking(true).unwrap();
    let mut de_novo = None;
    assert!(
        ate(Duration::from_secs(3), || {
            de_novo = ouvinte.accept().ok();
            de_novo.is_some()
        }),
        "o cliente seguiu na ligação que mandou uma linha de 70 kB sem fim"
    );
    let (mut conexao, _) = de_novo.unwrap();
    conexao.set_nonblocking(false).unwrap();
    conexao
        .write_all(estado(true, 50.0, -5.0, [0.5, 0.25, 90.0]).as_bytes())
        .unwrap();
    assert!(
        ate(Duration::from_secs(2), || amostras.lock().unwrap().len()
            == 5),
        "a amostra de depois da linha sem fim não chegou"
    );

    // Mandado parar, o cliente sai, mesmo com a ligação de pé.
    parar.store(true, Ordering::Relaxed);
    assert!(
        ate(Duration::from_secs(2), || cliente.is_finished()),
        "o cliente não parou"
    );
}
