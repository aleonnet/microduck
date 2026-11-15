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
