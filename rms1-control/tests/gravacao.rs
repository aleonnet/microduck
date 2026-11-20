//! A comparação que prova que simulador e robô receberam os mesmos quadros.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rms1_control::corpo::{Corpo, CorpoFalso};
use rms1_control::gravacao::{Gravando, diferencas};
use rms1_control::{FonteId, Intencoes, Laco, Saude};
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
        intencoes.pedir_twist(FonteId::SEM_NOME, [vx, 0.0, 0.0]);
        t = Instant::now().max(t + Duration::from_millis(1));
        laco.ciclo(t, false);
    }
    intencoes.pedir_torre(FonteId::SEM_NOME, 0.2, 0.1);
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
        intencoes.pedir_torre(FonteId::SEM_NOME, 0.2, 0.1);
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
    let bytes: Vec<u8> = ler_recebido(&recebido)
        .unwrap()
        .into_iter()
        .flat_map(|(_, b)| b)
        .collect();
    assert_eq!(bytes, fluxo, "os bytes, na ordem");
    assert_eq!(Leitor::new().empurrar(&bytes), vistos);
}

/// Critério 22, pelo caminho inteiro: o laço real, com o corpo respondendo como o controlador, e o reprocessamento do recebido
/// contando os mesmos pacotes, com o mesmo cálculo. No fluxo vão pacotes da mensagem da DJI e da nossa antes de a assinatura
/// valer, e de outra mensagem depois: o laço não os conta, e o reprocessamento também não pode contar.
#[test]
fn o_reprocessamento_do_recebido_conta_os_pacotes_que_o_laco_contou() {
    use rms1_control::Enlace;
    use rms1_control::estimativa::de_recebido;
    use rms1_control::gravacao::ler_recebido;
    use rms1_proto::barramento::id;
    use rms1_proto::{PedeAck, Quadro};
    use rms1d_params::TipoCorpo;

    let pacote = |mensagem: u8, seq: u16, relogio_ms: u32| {
        let mut d = vec![0u8; 163];
        d[1] = mensagem;
        d[2..6].copy_from_slice(&relogio_ms.to_le_bytes());
        // Rodas a 100 rpm, para o cálculo ter o que calcular.
        d[58..60].copy_from_slice(&100i16.to_le_bytes());
        Quadro {
            origem: 0x03,
            destino: 0x09,
            seq,
            resposta: false,
            pede: PedeAck::Agora,
            cmd_set: 0x48,
            cmd_id: 0x08,
            dados: d,
        }
    };
    let resposta = |cmd: (u8, u8)| Quadro {
        origem: 0x03,
        destino: 0x09,
        seq: 0,
        resposta: true,
        pede: PedeAck::Nada,
        cmd_set: cmd.0,
        cmd_id: cmd.1,
        dados: vec![0],
    };

    let mut p = Params::default();
    p.corpo.tipo = TipoCorpo::Serial;
    // Em tempo real, porque a gravação usa o relógio de verdade: a observação e as esperas, encurtadas.
    p.telemetria.janela_ms = 60;
    p.telemetria.espera_ms = 200;
    let d = tempfile_dir();
    let (enviado, recebido) = (d.join("rl-enviado"), d.join("rl-recebido"));
    let corpo = CorpoFalso::default();
    let gravando = Gravando::new(corpo.clone(), &enviado)
        .unwrap()
        .com_recebido(&recebido)
        .unwrap();
    let saude = Arc::new(Saude::default());
    let mut laco = Laco::new(gravando, &p, Arc::new(Intencoes::default()), saude.clone());
    let manda = |q: &Quadro| corpo.a_ler.lock().unwrap().extend(q.montar().unwrap());
    let mut ciclo = || {
        std::thread::sleep(Duration::from_millis(5));
        laco.ciclo(Instant::now(), false);
    };
    let enlace = |s: &Saude| s.amostra().map(|a| a.enlace);

    ciclo();
    // Durante a observação: a mensagem 0 e a nossa, que o laço vê e não conta.
    for i in 0..4 {
        manda(&pacote(i % 2, i.into(), 1000 + 20 * u32::from(i)));
        ciclo();
    }
    for _ in 0..200 {
        ciclo();
        match enlace(&saude) {
            Some(Enlace::Registrando) => {
                // Folga para o fim da observação no reprocessamento, que conta do início da gravação.
                for _ in 0..4 {
                    ciclo();
                }
                manda(&resposta(id::REGISTRAR));
            }
            Some(Enlace::Cancelando) => manda(&resposta(id::CANCELAR)),
            Some(Enlace::Assinando) => manda(&resposta(id::ASSINAR)),
            Some(Enlace::Esperando) => break,
            _ => {}
        }
    }
    assert_eq!(
        enlace(&saude),
        Some(Enlace::Esperando),
        "a assinatura valeu"
    );
    for i in 0..30u16 {
        manda(&pacote(1, 100 + i, 2000 + 20 * u32::from(i)));
        if i % 5 == 0 {
            manda(&pacote(2, 500 + i, 2000 + 20 * u32::from(i)));
        }
        ciclo();
    }
    let vivo = saude.amostra().unwrap();
    assert_eq!(
        vivo.pacotes, 30,
        "o laço conta só os da nossa mensagem, depois de assinar"
    );
    drop(laco);

    let r = de_recebido(&ler_recebido(&recebido).unwrap(), &p);
    assert_eq!(
        r.linhas.len() as u64,
        vivo.pacotes,
        "os mesmos pacotes que o laço contou"
    );
    assert_eq!(
        r.linhas.last().map(|l| l.calculo),
        vivo.estimativa,
        "o mesmo cálculo do último pacote"
    );
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

/// Um disco de mentira: demora em cada escrita, ou recusa como disco cheio, e guarda o que aceitou.
#[derive(Clone, Default)]
struct DiscoFalso {
    demora: Duration,
    cheio: bool,
    escrito: Arc<std::sync::Mutex<Vec<u8>>>,
}

impl std::io::Write for DiscoFalso {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        std::thread::sleep(self.demora);
        if self.cheio {
            return Err(std::io::Error::other("No space left on device"));
        }
        self.escrito.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn um_quadro() -> Vec<u8> {
    rms1_proto::barramento::Pedido::Responder { seq: 7 }
        .quadro(0)
        .montar()
        .unwrap()
}

/// Adendo 2, critério 73: o disco lento não segura o ciclo. Em 2026-11-15 o disco do robô encheu, a escrita era feita dentro
/// do ciclo, e o laço caiu de 100 para 2 a 4 Hz.
#[test]
fn disco_lento_nao_segura_o_ciclo() {
    let lento = DiscoFalso {
        demora: Duration::from_millis(300),
        ..DiscoFalso::default()
    };
    let corpo = CorpoFalso::default();
    let quadro = um_quadro();
    let mut g = Gravando::com_escritores(
        corpo.clone(),
        Box::new(lento.clone()),
        Some(Box::new(lento)),
    );
    let mut buf = [0u8; 64];
    let inicio = Instant::now();
    for _ in 0..200 {
        g.enviar(&quadro).unwrap();
        corpo.a_ler.lock().unwrap().extend_from_slice(&quadro);
        assert!(g.ler(&mut buf).unwrap() > 0);
    }
    let gasto = inicio.elapsed();
    assert!(
        gasto < Duration::from_millis(100),
        "o ciclo esperou o disco: 200 envios e 200 leituras levaram {gasto:?}"
    );
    assert_eq!(
        corpo.quadros().len(),
        200,
        "todos os quadros foram ao corpo"
    );
}

/// Adendo 2, critério 74: o disco cheio não vira falha de envio. O quadro foi ao corpo; dizer que não foi fazia a saúde
/// acusar "o corpo não recebe".
#[test]
fn disco_cheio_nao_vira_falha_de_envio() {
    let cheio = DiscoFalso {
        cheio: true,
        ..DiscoFalso::default()
    };
    let corpo = CorpoFalso::default();
    let gravando = Gravando::com_escritores(
        corpo.clone(),
        Box::new(cheio.clone()),
        Some(Box::new(cheio)),
    );
    let intencoes = Arc::new(Intencoes::default());
    let saude = Arc::new(Saude::default());
    let mut laco = Laco::new(
        gravando,
        &Params::default(),
        intencoes.clone(),
        saude.clone(),
    );
    intencoes.pedir_armar(true);
    let mut t = Instant::now();
    for _ in 0..50 {
        intencoes.pedir_twist(FonteId::SEM_NOME, [0.5, 0.0, 0.0]);
        t += Duration::from_millis(10);
        laco.ciclo(t, false);
    }
    let falhados = saude
        .erros_seguidos
        .load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        falhados, 0,
        "a falha da gravação foi contada como {falhados} envios falhados ao corpo"
    );
    assert!(corpo.quadros().len() >= 100, "os quadros foram ao corpo");
}

/// Adendo 2, critério 75: a gravação para no limite, e o ciclo segue enviando.
#[test]
fn a_gravacao_para_no_limite() {
    let d = tempfile_dir();
    let (enviado, recebido) = (d.join("lim-enviado"), d.join("lim-recebido"));
    let corpo = CorpoFalso::default();
    let quadro = um_quadro();
    let limite = 2000u64;
    let mut g = Gravando::new(corpo.clone(), &enviado)
        .unwrap()
        .com_recebido(&recebido)
        .unwrap()
        .com_limite(limite);
    let mut buf = [0u8; 64];
    for _ in 0..200 {
        g.enviar(&quadro).unwrap();
        corpo.a_ler.lock().unwrap().extend_from_slice(&quadro);
        assert!(g.ler(&mut buf).unwrap() > 0);
    }
    drop(g);
    let textos = [&enviado, &recebido].map(|a| std::fs::read_to_string(a).unwrap());
    let soma: u64 = textos.iter().map(|t| t.len() as u64).sum();
    assert!(
        soma > limite / 2 && soma <= limite,
        "os dois arquivos somam {soma} bytes, com o limite em {limite}"
    );
    assert!(
        textos.iter().all(|t| t.is_empty() || t.ends_with('\n')),
        "só linhas inteiras"
    );
    assert_eq!(corpo.quadros().len(), 200, "o envio não parou no limite");
}

/// Adendo 2, critério 76: com a fila cheia, linhas inteiras são descartadas e contadas, e o arquivo só tem linhas inteiras.
#[test]
fn fila_cheia_perde_linhas_inteiras_e_conta() {
    let lento = DiscoFalso {
        demora: Duration::from_millis(20),
        ..DiscoFalso::default()
    };
    let escrito = lento.escrito.clone();
    let corpo = CorpoFalso::default();
    let quadro = um_quadro();
    let mut g = Gravando::com_escritores(corpo, Box::new(lento), None);
    let perdidas = g.perdidas();
    for _ in 0..5000 {
        g.enviar(&quadro).unwrap();
    }
    drop(g);
    let n = perdidas.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        n > 0,
        "com o disco a 20 ms por escrita, 5000 linhas não cabem na fila"
    );
    let texto = String::from_utf8(escrito.lock().unwrap().clone()).unwrap();
    assert!(texto.ends_with('\n'), "a última linha está inteira");
    let linhas: Vec<&str> = texto.lines().collect();
    assert_eq!(
        linhas.len() as u64,
        5000 - n,
        "as gravadas mais as descartadas são as enviadas"
    );
    for l in linhas {
        let (ms, h) = l.split_once(' ').expect("ms e hexadecimal");
        ms.parse::<u64>().expect("milissegundos");
        assert_eq!(h.len(), quadro.len() * 2, "linha cortada: {l}");
    }
}
