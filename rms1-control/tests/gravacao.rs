//! A comparação que prova que simulador e robô receberam os mesmos quadros.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rms1_control::corpo::{Corpo, CorpoFalso};
use rms1_control::gravacao::{Gravando, diferencas};
use rms1_control::{Intencoes, Laco, Saude};
use rms1d_params::Params;

fn gravar(caminho: &std::path::Path, ciclos_por_pedido: usize, vx: f64) {
    let corpo: Box<dyn Corpo> = Box::new(CorpoFalso::default());
    let gravando = Gravando::new(corpo, caminho).unwrap();
    let intencoes = Arc::new(Intencoes::default());
    let mut laco = Laco::new(
        gravando,
        &Params::default(),
        intencoes.clone(),
        Arc::new(Saude::default()),
    );
    intencoes.pedir_armar(true);
    let mut t = Instant::now();
    for _ in 0..ciclos_por_pedido {
        intencoes.pedir_twist([vx, 0.0, 0.0]);
        t = Instant::now().max(t + Duration::from_millis(1));
        laco.ciclo(t, false);
    }
    intencoes.pedir_torre(0.2, 0.1);
    laco.ciclo(t, false);
    intencoes.pedir_armar(false);
    laco.ciclo(t, false);
}

/// Duração diferente, mesma sequência de conteúdos: igual. É o caso de simulador e robô.
#[test]
fn o_mesmo_roteiro_com_tempos_diferentes_e_igual() {
    let d = tempfile_dir();
    let (a, b) = (d.join("a"), d.join("b"));
    gravar(&a, 3, 1.0);
    gravar(&b, 30, 1.0);
    assert_eq!(diferencas(&a, &b).unwrap(), Vec::<String>::new());
}

#[test]
fn um_comando_diferente_aparece_na_comparacao() {
    let d = tempfile_dir();
    let (a, b) = (d.join("a"), d.join("b"));
    gravar(&a, 3, 1.0);
    gravar(&b, 3, 0.5);
    let dif = diferencas(&a, &b).unwrap();
    assert_eq!(dif.len(), 1, "{dif:?}");
    assert!(dif[0].contains("0x3f/0x20"), "{dif:?}");
}

/// O identificador de tarefa muda a cada envio: dois roteiros de durações diferentes têm de dar iguais.
#[test]
fn o_identificador_de_tarefa_nao_conta_como_diferenca() {
    let d = tempfile_dir();
    let (a, b) = (d.join("ta"), d.join("tb"));
    for (arquivo, repete) in [(&a, 2), (&b, 50)] {
        let corpo: Box<dyn Corpo> = Box::new(CorpoFalso::default());
        let gravando = Gravando::new(corpo, arquivo).unwrap();
        let intencoes = Arc::new(Intencoes::default());
        let mut laco = Laco::new(
            gravando,
            &Params::default(),
            intencoes.clone(),
            Arc::new(Saude::default()),
        );
        intencoes.pedir_armar(true);
        intencoes.pedir_torre(0.2, 0.1);
        let mut t = Instant::now();
        for _ in 0..repete {
            t += Duration::from_millis(200);
            laco.ciclo(t, false);
        }
    }
    assert_eq!(diferencas(&a, &b).unwrap(), Vec::<String>::new());
}

/// A telemetria depende do que o corpo devolve: simulador e corpo falso não têm de mandar os mesmos pedidos. Os comandos, sim.
#[test]
fn a_telemetria_fica_fora_da_comparacao_e_os_comandos_nao() {
    use rms1_proto::barramento::Pedido;
    let d = tempfile_dir();
    let (a, b, c) = (d.join("za"), d.join("zb"), d.join("zc"));
    gravar(&a, 3, 1.0);
    gravar(&b, 3, 1.0);
    gravar(&c, 3, 0.5);
    let extra = Pedido::RegistrarNo.quadro(0).montar().unwrap();
    let linha = format!(
        "1 {}\n",
        extra.iter().map(|x| format!("{x:02x}")).collect::<String>()
    );
    let mut t = std::fs::read_to_string(&b).unwrap();
    t.push_str(&linha);
    std::fs::write(&b, t).unwrap();
    assert_eq!(diferencas(&a, &b).unwrap(), Vec::<String>::new());
    assert_eq!(diferencas(&a, &c).unwrap().len(), 1);
}

/// Um corpo que devolve bytes em pedaços que cortam quadros ao meio, como a serial.
struct CorpoQueDevolve {
    pedacos: std::collections::VecDeque<Vec<u8>>,
}

impl Corpo for CorpoQueDevolve {
    fn enviar(&mut self, _: &[u8]) -> std::io::Result<()> {
        Ok(())
    }
    fn ler(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(p) = self.pedacos.pop_front() else {
            return Ok(0);
        };
        buf[..p.len()].copy_from_slice(&p);
        Ok(p.len())
    }
}

/// Critério 22: cada leitura do corpo é gravada, e a gravação dá os mesmos quadros que a leitura viu, inteiros.
#[test]
fn o_recebido_gravado_da_os_mesmos_quadros_que_a_leitura() {
    use rms1_control::gravacao::ler_recebido;
    use rms1_proto::Leitor;
    use rms1_proto::barramento::Pedido;
    let mut fluxo = Vec::new();
    for seq in 0..40u16 {
        fluxo.extend(Pedido::Responder { seq }.quadro(0).montar().unwrap());
        fluxo.push(0x00); // um byte solto entre quadros, como na serial
    }
    let pedacos = fluxo.chunks(7).map(<[u8]>::to_vec).collect();
    let d = tempfile_dir();
    let (enviado, recebido) = (d.join("re-enviado"), d.join("re-recebido"));
    let mut g = Gravando::new(CorpoQueDevolve { pedacos }, &enviado)
        .unwrap()
        .com_recebido(&recebido)
        .unwrap();
    let (mut leitor, mut vistos) = (Leitor::new(), Vec::new());
    let mut buf = [0u8; 64];
    loop {
        let n = g.ler(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        vistos.extend(leitor.empurrar(&buf[..n]));
    }
    drop(g);
    assert_eq!(vistos.len(), 40);
    let bytes = ler_recebido(&recebido).unwrap();
    assert_eq!(bytes, fluxo, "os bytes, na ordem");
    assert_eq!(Leitor::new().empurrar(&bytes), vistos);
}

/// Dois nadas iguais não provam nada.
#[test]
fn gravacao_vazia_nao_e_igual_a_nada() {
    let d = tempfile_dir();
    let (a, b) = (d.join("va"), d.join("vb"));
    std::fs::write(&a, "").unwrap();
    gravar(&b, 3, 1.0);
    let e = diferencas(&a, &b).unwrap_err().to_string();
    assert!(e.contains("não tem nenhum quadro"), "{e}");
    let e = diferencas(&a, &a).unwrap_err().to_string();
    assert!(e.contains("não tem nenhum quadro"), "{e}");
}

fn tempfile_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "rms1-gravacao-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}
