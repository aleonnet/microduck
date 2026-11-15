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
