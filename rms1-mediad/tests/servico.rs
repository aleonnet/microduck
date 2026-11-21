//! O socket do serviço, além do quadro: o som que chega por `robot.sound` e o que o robô ouviu, por assinatura.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rms1_mediad::alto_falante::{Saida, Tocador, Tocando};
use rms1_mediad::microfone::Ouvido;
use rms1_mediad::quadros::Quadros;
use rms1_mediad::servidor::{self, Servico};
use rms1_mediad::trava::Trava;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Uma saída que só anota o que foi pedido a ela.
#[derive(Clone, Default)]
struct Anota(Arc<Mutex<Vec<String>>>);

struct Calado;

impl Tocando for Calado {
    fn terminou(&mut self) -> bool {
        false
    }
    fn cortar(&mut self) {}
}

impl Saida for Anota {
    fn tocar_arquivo(&mut self, wav: &Path) -> std::io::Result<Box<dyn Tocando>> {
        let pasta = wav.parent().unwrap().file_name().unwrap().to_str().unwrap();
        self.0.lock().unwrap().push(pasta.to_owned());
        Ok(Box::new(Calado))
    }
    fn abrir_fluxo(
        &mut self,
        _hz: u32,
    ) -> std::io::Result<(Box<dyn Write + Send>, Box<dyn Tocando>)> {
        self.0.lock().unwrap().push("fluxo".to_owned());
        Ok((Box::new(std::io::sink()), Box::new(Calado)))
    }
}

fn montar() -> (tempfile::TempDir, Anota, Arc<Servico>) {
    let pasta = tempfile::tempdir().unwrap();
    for tag in ["chirp", "greet"] {
        std::fs::create_dir_all(pasta.path().join(tag)).unwrap();
        std::fs::write(pasta.path().join(tag).join("a.wav"), b"RIFF").unwrap();
    }
    let anota = Anota::default();
    let tocador = Tocador::novo(
        pasta.path().to_owned(),
        Box::new(anota.clone()),
        Trava::nova(Duration::from_millis(800)),
    );
    (pasta, anota, Servico::novo(Quadros::default(), tocador))
}

async fn linha(leitor: &mut BufReader<UnixStream>) -> serde_json::Value {
    let mut texto = String::new();
    tokio::time::timeout(Duration::from_secs(5), leitor.read_line(&mut texto))
        .await
        .expect("uma linha em 5 s")
        .unwrap();
    serde_json::from_str(&texto).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn robot_sound_toca_por_requisicao_e_por_notificacao() {
    let (_pasta, anota, servico) = montar();
    let (cliente, servidor_) = UnixStream::pair().unwrap();
    tokio::spawn(servidor::atender(servidor_, servico));
    let mut c = BufReader::new(cliente);
    // A notificação, que é como o `rms1d` repassa, não tem resposta; a requisição seguinte tem.
    c.get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"robot.sound\",\"params\":{\"tag\":\"chirp\"}}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"robot.sound\",\"params\":{\"tag\":\"greet\"}}\n")
        .await
        .unwrap();
    let r = linha(&mut c).await;
    assert_eq!(
        (r["id"].as_u64(), r["result"]["accepted"].as_bool()),
        (Some(2), Some(true)),
        "{r}"
    );
    assert_eq!(*anota.0.lock().unwrap(), ["chirp", "greet"]);
    // Um som que o contrato não tem é recusado pelo nome, e nada toca.
    c.get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"robot.sound\",\"params\":{\"tag\":\"latido\"}}\n")
        .await
        .unwrap();
    let r = linha(&mut c).await;
    assert!(
        r["error"]["message"].as_str().unwrap().contains("latido"),
        "{r}"
    );
    assert_eq!(anota.0.lock().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn quem_assina_recebe_o_que_o_robo_ouviu() {
    let (_pasta, _anota, servico) = montar();
    let (cliente, servidor_) = UnixStream::pair().unwrap();
    tokio::spawn(servidor::atender(servidor_, servico.clone()));
    let mut c = BufReader::new(cliente);
    c.get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"s1.media.subscribe\",\"params\":{\"heard\":true}}\n")
        .await
        .unwrap();
    assert_eq!(linha(&mut c).await["result"]["accepted"], true);
    servico.ouvidos.send(Ouvido::Barulho).unwrap();
    servico.ouvidos.send(Ouvido::Voz).unwrap();
    for tipo in ["noise", "voice"] {
        let n = linha(&mut c).await;
        assert_eq!(
            (n["method"].as_str(), n["params"]["kind"].as_str()),
            (Some("s1.audio.heard"), Some(tipo)),
            "{n}"
        );
    }
    // Quem não assinou não recebe nada: sem assinante, o envio não tem para quem ir.
    let (_p2, _a2, outro) = montar();
    assert!(outro.ouvidos.send(Ouvido::Voz).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn quem_assina_recebe_as_deteccoes_e_o_estado_mostra_a_percepcao() {
    let (_pasta, _anota, servico) = montar();
    let (cliente, servidor_) = UnixStream::pair().unwrap();
    tokio::spawn(servidor::atender(servidor_, servico.clone()));
    let mut c = BufReader::new(cliente);
    // Sem assinatura, a detecção não tem para quem ir.
    let deteccao = rms1_mediad::percepcao::notificacao(&[], 1280, 720, 45.0);
    assert!(servico.deteccoes.send(deteccao.as_str().into()).is_err());
    c.get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"s1.media.subscribe\",\"params\":{\"detections\":true}}\n")
        .await
        .unwrap();
    assert_eq!(linha(&mut c).await["result"]["accepted"], true);
    servico.deteccoes.send(deteccao.as_str().into()).unwrap();
    let n = linha(&mut c).await;
    assert_eq!(
        (n["method"].as_str(), n["params"]["width"].as_u64()),
        (Some("media.detections"), Some(1280)),
        "{n}"
    );
    // Quem assinou só as detecções não recebe o que o robô ouviu.
    assert!(servico.ouvidos.send(Ouvido::Voz).is_err());

    // O estado mostra a percepção: sem estação, diz por quê.
    servico.percepcao.lock().unwrap().idle_reason =
        Some("nenhuma estação se anunciou na rede".to_owned());
    c.get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"s1.media.status\"}\n")
        .await
        .unwrap();
    let r = linha(&mut c).await;
    let p = &r["result"]["perception"];
    assert_eq!(
        p["idle_reason"], "nenhuma estação se anunciou na rede",
        "{r}"
    );
    assert_eq!(
        (p["frames_sent"].as_u64(), p["station"].is_null()),
        (Some(0), true),
        "{r}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_previa_entrega_o_quadro_reduzido_em_rgb_e_a_conexao_fica() {
    use rms1_mediad::quadros::Quadro;
    use tokio::io::AsyncReadExt;
    let (_pasta, _anota, servico) = montar();
    // Uma câmera de mentira: a cada pedido, um quadro NV12 de 1280 por 720, branco em cima e preto embaixo.
    let q = servico.quadros.clone();
    std::thread::spawn(move || {
        loop {
            if q.levar_pedido() {
                let (l, a) = (1280usize, 720usize);
                let mut dados = vec![128u8; l * a * 3 / 2];
                dados[..l * a / 2].fill(235);
                dados[l * a / 2..l * a].fill(16);
                q.entregar(Quadro {
                    largura: l as u32,
                    altura: a as u32,
                    formato: "NV12",
                    capturado_em: std::time::SystemTime::now(),
                    dados,
                });
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    let (cliente, servidor_) = UnixStream::pair().unwrap();
    tokio::spawn(servidor::atender(servidor_, servico));
    let mut c = BufReader::new(cliente);
    // Dois pedidos pela mesma conexão: o padrão, e uma largura pedida.
    for (id, pedido, l, a) in [
        (
            1u64,
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"s1.media.preview\"}\n",
            640usize,
            360usize,
        ),
        (
            2,
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"s1.media.preview\",\"params\":{\"width\":320}}\n",
            320,
            180,
        ),
    ] {
        c.get_mut().write_all(pedido.as_bytes()).await.unwrap();
        let r = linha(&mut c).await;
        let h = &r["result"];
        assert_eq!(
            (
                r["id"].as_u64(),
                h["width"].as_u64(),
                h["height"].as_u64(),
                h["format"].as_str(),
                h["bytes"].as_u64()
            ),
            (
                Some(id),
                Some(l as u64),
                Some(a as u64),
                Some("RGB"),
                Some((l * a * 3) as u64)
            ),
            "{r}"
        );
        let mut rgb = vec![0u8; l * a * 3];
        tokio::time::timeout(Duration::from_secs(5), c.read_exact(&mut rgb))
            .await
            .expect("os bytes do quadro em 5 s")
            .unwrap();
        assert_eq!(&rgb[..3], [255, 255, 255], "o de cima é branco");
        assert_eq!(&rgb[rgb.len() - 3..], [0, 0, 0], "o de baixo é preto");
    }
}

/// Uma câmera de mentira: a cada pedido, um quadro NV12 de 1280 por 720, branco em cima e preto embaixo.
fn camera_de_mentira(servico: &Arc<Servico>) {
    use rms1_mediad::quadros::Quadro;
    let q = servico.quadros.clone();
    std::thread::spawn(move || {
        loop {
            if q.levar_pedido() {
                let (l, a) = (1280usize, 720usize);
                let mut dados = vec![128u8; l * a * 3 / 2];
                dados[..l * a / 2].fill(235);
                dados[l * a / 2..l * a].fill(16);
                q.entregar(Quadro {
                    largura: l as u32,
                    altura: a as u32,
                    formato: "NV12",
                    capturado_em: std::time::SystemTime::now(),
                    dados,
                });
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_porta_do_video_entrega_jpeg_pela_rede_e_so_deixa_olhar() {
    use tokio::io::AsyncReadExt;
    let (_pasta, anota, servico) = montar();
    camera_de_mentira(&servico);
    let porta = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endereco = porta.local_addr().unwrap();
    tokio::spawn(servidor::servir_tcp(porta, servico));
    let mut c = BufReader::new(tokio::net::TcpStream::connect(endereco).await.unwrap());
    // Dois quadros pela mesma conexão, em JPEG: bem menores que o RGB, e cada um um JPEG inteiro.
    for id in [1u64, 2] {
        let pedido = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"s1.media.preview\",\"params\":{{\"format\":\"jpeg\"}}}}\n"
        );
        c.get_mut().write_all(pedido.as_bytes()).await.unwrap();
        let mut texto = String::new();
        tokio::time::timeout(Duration::from_secs(5), c.read_line(&mut texto))
            .await
            .expect("o cabeçalho em 5 s")
            .unwrap();
        let r: serde_json::Value = serde_json::from_str(&texto).unwrap();
        let h = &r["result"];
        let n = h["bytes"].as_u64().unwrap() as usize;
        assert_eq!(
            (
                h["width"].as_u64(),
                h["height"].as_u64(),
                h["format"].as_str()
            ),
            (Some(640), Some(360), Some("JPEG")),
            "{r}"
        );
        assert!(
            n < 640 * 360 * 3 / 10,
            "o JPEG saiu grande demais: {n} bytes"
        );
        let mut jpeg = vec![0u8; n];
        tokio::time::timeout(Duration::from_secs(5), c.read_exact(&mut jpeg))
            .await
            .expect("os bytes do quadro em 5 s")
            .unwrap();
        assert_eq!(
            (&jpeg[..2], &jpeg[n - 2..]),
            (&[0xFF, 0xD8][..], &[0xFF, 0xD9][..]),
            "não é um JPEG inteiro"
        );
        // O que o JPEG guarda é o quadro: branco em cima, preto embaixo.
        let aberto = image::load_from_memory(&jpeg).unwrap().to_rgb8();
        assert_eq!((aberto.width(), aberto.height()), (640, 360));
        assert!(
            aberto.get_pixel(320, 20).0.iter().all(|&v| v > 240),
            "o de cima não é branco"
        );
        assert!(
            aberto.get_pixel(320, 340).0.iter().all(|&v| v < 15),
            "o de baixo não é preto"
        );
    }
    // Pela porta do vídeo ninguém toca som nem muda o volume: só se olha.
    for metodo in [
        "robot.sound",
        "s1.media.volume",
        "s1.media.subscribe",
        "media.frame",
    ] {
        let pedido = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"{metodo}\",\"params\":{{\"tag\":\"chirp\",\"volume\":10}}}}\n"
        );
        c.get_mut().write_all(pedido.as_bytes()).await.unwrap();
        let r = linha_de(&mut c).await;
        assert!(
            r["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("video port"),
            "{metodo} passou pela porta do vídeo: {r}"
        );
    }
    assert!(
        anota.0.lock().unwrap().is_empty(),
        "um som tocou pela porta do vídeo"
    );
    // O estado, que não muda nada, passa.
    c.get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"s1.media.status\"}\n")
        .await
        .unwrap();
    assert!(linha_de(&mut c).await["result"]["perception"].is_object());
}

async fn linha_de<C: tokio::io::AsyncBufRead + Unpin>(leitor: &mut C) -> serde_json::Value {
    let mut texto = String::new();
    tokio::time::timeout(Duration::from_secs(5), leitor.read_line(&mut texto))
        .await
        .expect("uma linha em 5 s")
        .unwrap();
    serde_json::from_str(&texto).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn quem_assina_a_conversa_recebe_o_que_a_estacao_respondeu() {
    let (_pasta, _anota, servico) = montar();
    let (cliente, servidor_) = UnixStream::pair().unwrap();
    tokio::spawn(servidor::atender(servidor_, servico.clone()));
    let mut c = BufReader::new(cliente);
    let resultado = r#"{"jsonrpc":"2.0","method":"s1.station.result","params":{"function":"pensar","delivers":"pedidos_validos","event":{"o":"fim","resultado":"feito"}}}"#;
    // Sem assinatura, o resultado não tem para quem ir.
    assert!(servico.resultados.send(resultado.into()).is_err());
    c.get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"s1.media.subscribe\",\"params\":{\"voice\":true}}\n")
        .await
        .unwrap();
    assert_eq!(linha(&mut c).await["result"]["accepted"], true);
    servico.resultados.send(resultado.into()).unwrap();
    let n = linha(&mut c).await;
    assert_eq!(
        (
            n["method"].as_str(),
            n["params"]["event"]["resultado"].as_str()
        ),
        (Some("s1.station.result"), Some("feito")),
        "{n}"
    );
    // Quem assinou só a conversa não recebe as caixas nem o que o robô ouviu.
    assert!(servico.ouvidos.send(Ouvido::Voz).is_err());
    assert!(servico.deteccoes.send("{}".into()).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn quem_assina_os_pedidos_recebe_o_que_a_estacao_manda_cumprir_e_diz_o_desfecho() {
    let (_pasta, _anota, servico) = montar();
    let (devolver, desfechos) = std::sync::mpsc::channel::<String>();
    let (cliente, servidor_) = UnixStream::pair().unwrap();
    tokio::spawn(servidor::atender(servidor_, servico.clone()));
    let mut c = BufReader::new(cliente);
    let pedido = r#"{"jsonrpc":"2.0","method":"s1.station.request","params":{"request":41,"tool":"move","params":{"direction":"forward","meters":0.5}}}"#;
    // Sem assinatura, o pedido não tem para quem ir: é como o serviço sabe que não há agente.
    assert!(servico.pedidos.send(pedido.into()).is_err());
    c.get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"s1.media.subscribe\",\"params\":{\"requests\":true}}\n")
        .await
        .unwrap();
    assert_eq!(linha(&mut c).await["result"]["accepted"], true);
    servico.pedidos.send(pedido.into()).unwrap();
    let n = linha(&mut c).await;
    assert_eq!(
        (n["method"].as_str(), n["params"]["tool"].as_str()),
        (Some("s1.station.request"), Some("move")),
        "{n}"
    );
    // Sem a ligação com a estação, o desfecho não tem para onde ir, e quem o disse fica sabendo.
    let desfecho = b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"s1.station.outcome\",\"params\":{\"request\":41,\"tool\":\"move\",\"outcome\":\"cumprida\",\"reason\":\"andei 0.50 m\"}}\n";
    c.get_mut().write_all(desfecho).await.unwrap();
    let r = linha(&mut c).await;
    assert_eq!(r["result"]["accepted"], false, "{r}");
    // Com ela, o desfecho vai como a estação o lê.
    servico.desfechos.set(devolver).unwrap();
    c.get_mut().write_all(desfecho).await.unwrap();
    assert_eq!(linha(&mut c).await["result"]["accepted"], true);
    let d: serde_json::Value =
        serde_json::from_str(&desfechos.recv_timeout(Duration::from_secs(2)).unwrap()).unwrap();
    assert_eq!(
        d,
        serde_json::json!({"pedido": 41, "ferramenta": "move", "como": "cumprida", "motivo": "andei 0.50 m"})
    );
    // O desfecho sem o número do pedido, ou com um desfecho que não existe, é recusado dizendo o que falta.
    for (errado, palavra) in [
        (
            "{\"tool\":\"move\",\"outcome\":\"cumprida\",\"reason\":\"\"}",
            "request",
        ),
        (
            "{\"request\":41,\"tool\":\"move\",\"outcome\":\"talvez\",\"reason\":\"\"}",
            "outcome",
        ),
    ] {
        let pedido = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"s1.station.outcome\",\"params\":{errado}}}\n"
        );
        c.get_mut().write_all(pedido.as_bytes()).await.unwrap();
        let r = linha(&mut c).await;
        assert!(
            r["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains(palavra)),
            "{palavra}: {r}"
        );
    }
    assert!(desfechos.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn quem_assina_o_mundo_recebe_as_duas_linhas_e_quem_assina_so_as_deteccoes_nao_recebe_nada_de_novo()
 {
    use rms1_mundo::{Corpo, Mundo};
    use rms1d_params::{Fisica, MidiaMundo};
    let (_pasta, _anota, servico) = montar();
    // As duas linhas como a camada de mundo as monta: aqui, a de um robô com a lente ainda por medir.
    let f = Fisica::da_tabela();
    let corpo = Corpo {
        largura_m: f.largura_m,
        frente_m: f.frente_m,
    };
    let mundo = Mundo::novo(None, corpo, MidiaMundo::default()).unwrap();
    let do_mundo = serde_json::json!({"jsonrpc": "2.0", "method": "s1.world", "params": mundo.agora(1.0).linha()})
        .to_string();
    let da_grade = serde_json::json!({"jsonrpc": "2.0", "method": "s1.world.grid", "params": mundo.linha_da_grade(1.0)})
        .to_string();
    // Sem assinatura, as linhas não têm para quem ir: é por aí que a camada de mundo sabe que não precisa montá-las.
    assert!(servico.mundo.send(do_mundo.as_str().into()).is_err());
    assert!(
        servico
            .grade_do_mundo
            .send(da_grade.as_str().into())
            .is_err()
    );

    let (cliente, servidor_) = UnixStream::pair().unwrap();
    tokio::spawn(servidor::atender(servidor_, servico.clone()));
    let mut c = BufReader::new(cliente);
    c.get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"s1.media.subscribe\",\"params\":{\"world\":true,\"world_grid\":true}}\n")
        .await
        .unwrap();
    assert_eq!(linha(&mut c).await["result"]["accepted"], true);
    // Outra conexão assina só as detecções, como o agente e as ferramentas de hoje.
    let (cliente, servidor_) = UnixStream::pair().unwrap();
    tokio::spawn(servidor::atender(servidor_, servico.clone()));
    let mut so_caixas = BufReader::new(cliente);
    so_caixas
        .get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"s1.media.subscribe\",\"params\":{\"detections\":true}}\n")
        .await
        .unwrap();
    assert_eq!(linha(&mut so_caixas).await["result"]["accepted"], true);

    assert_eq!(servico.mundo.send(do_mundo.as_str().into()).unwrap(), 1);
    let n = linha(&mut c).await;
    assert_eq!(
        (
            n["method"].as_str(),
            n["params"]["frame"].as_str(),
            n["params"]["perception"]["state"].as_str(),
            n.get("id")
        ),
        (Some("s1.world"), Some("body"), Some("no_camera"), None),
        "{n}"
    );
    assert_eq!(
        servico
            .grade_do_mundo
            .send(da_grade.as_str().into())
            .unwrap(),
        1
    );
    let n = linha(&mut c).await;
    assert_eq!(
        (
            n["method"].as_str(),
            n["params"]["frame"].as_str(),
            n["params"]["cells"].as_array().map(Vec::len)
        ),
        (Some("s1.world.grid"), Some("odom"), Some(0)),
        "{n}"
    );
    // Quem assinou só as detecções recebe a detecção, e nada do mundo antes dela.
    let deteccao = rms1_mediad::percepcao::notificacao(&[], 1280, 720, 45.0);
    servico.deteccoes.send(deteccao.as_str().into()).unwrap();
    assert_eq!(linha(&mut so_caixas).await["method"], "media.detections");
    // E quem assinou o mundo não recebe as detecções.
    servico.mundo.send(do_mundo.as_str().into()).unwrap();
    assert_eq!(linha(&mut c).await["method"], "s1.world");

    // Cada pedido diz as assinaturas todas: a que não vem é desligada.
    c.get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"s1.media.subscribe\",\"params\":{\"world\":true}}\n")
        .await
        .unwrap();
    assert_eq!(linha(&mut c).await["result"]["accepted"], true);
    assert!(
        servico
            .grade_do_mundo
            .send(da_grade.as_str().into())
            .is_err(),
        "a assinatura da grade segue ligada"
    );
    assert_eq!(servico.mundo.receiver_count(), 1);
}

/// `s1.media.depth`: a grade de profundidade que não vem da estação entra pelo socket, em bytes crus depois da linha do pedido.
/// É por onde a página do simulador entrega a profundidade de verdade da cena.
#[tokio::test(flavor = "multi_thread")]
async fn a_grade_de_profundidade_entra_pelo_socket_so_quando_o_servico_a_espera() {
    let (_pasta, _anota, servico) = montar();
    let (cliente, servidor_) = UnixStream::pair().unwrap();
    tokio::spawn(servidor::atender(servidor_, servico.clone()));
    let mut c = BufReader::new(cliente);
    let grade: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
    let pedido = |id: u32, bytes: usize| {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"s1.media.depth\",\"params\":{{\"bytes\":{bytes}}}}}\n"
        )
    };

    // Com a profundidade da estação, o pedido é recusado pelo motivo; a grade já foi lida, e a conexão segue no passo.
    c.get_mut()
        .write_all(pedido(1, grade.len()).as_bytes())
        .await
        .unwrap();
    c.get_mut().write_all(&grade).await.unwrap();
    let r = linha(&mut c).await;
    assert!(
        r["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("from the station")),
        "{r}"
    );
    c.get_mut()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"s1.media.status\"}\n")
        .await
        .unwrap();
    let r = linha(&mut c).await;
    assert_eq!(
        r["id"], 2,
        "depois da grade recusada, o pedido seguinte é atendido: {r}"
    );

    // Com quem a receba: a grade chega inteira, byte a byte, e a resposta diz que foi aceita.
    let recebidas: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    let guarda = recebidas.clone();
    let receber: servidor::GradeDeFora = Arc::new(move |carga: &[u8]| {
        if carga.len() < 16 {
            return Err("a grade tem menos que o cabeçalho".to_owned());
        }
        guarda.lock().unwrap().push(carga.to_vec());
        Ok(())
    });
    assert!(servico.grade_de_fora.set(receber).is_ok());
    c.get_mut()
        .write_all(pedido(3, grade.len()).as_bytes())
        .await
        .unwrap();
    c.get_mut().write_all(&grade).await.unwrap();
    let r = linha(&mut c).await;
    assert_eq!(
        (r["id"].as_u64(), r["result"]["accepted"].as_bool()),
        (Some(3), Some(true)),
        "{r}"
    );
    assert_eq!(recebidas.lock().unwrap().as_slice(), std::slice::from_ref(&grade));

    // A que quem recebe recusa volta com o motivo dele, e a conexão segue.
    c.get_mut()
        .write_all(pedido(4, 8).as_bytes())
        .await
        .unwrap();
    c.get_mut().write_all(&grade[..8]).await.unwrap();
    let r = linha(&mut c).await;
    assert!(
        r["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("menos que o cabeçalho")),
        "{r}"
    );
    assert_eq!(recebidas.lock().unwrap().len(), 1);

    // Sem tamanho, ou com um tamanho que passa do limite, não há como saber onde a grade acaba: recusa, e fecha.
    c.get_mut()
        .write_all(pedido(5, 2 << 20).as_bytes())
        .await
        .unwrap();
    let r = linha(&mut c).await;
    assert!(
        r["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("needs bytes")),
        "{r}"
    );
    let mut resto = String::new();
    let lidos = tokio::time::timeout(Duration::from_secs(2), c.read_line(&mut resto)).await;
    assert_eq!(
        lidos.expect("a conexão fecha").unwrap(),
        0,
        "a conexão tinha de fechar: {resto}"
    );
}
