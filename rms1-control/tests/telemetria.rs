//! A sequência da assinatura e o retrato do corpo, pelos quadros que entram e saem, como o robô os vê.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rms1_control::telemetria::{Enlace, Medidas, Passo};
use rms1_control::{CorpoFalso, Intencoes, Laco, Saude};
use rms1_proto::barramento::{Layout, Pacote, id};
use rms1_proto::{PedeAck, Quadro};
use rms1d_params::{Params, TipoCorpo};

const PARADO: &[u8] = include_bytes!("../../s1-duml/tests/fixtures/dds_push_176_real.bin");

struct Banca {
    corpo: CorpoFalso,
    saude: Arc<Saude>,
    laco: Laco<CorpoFalso>,
    agora: Instant,
}

fn banca() -> Banca {
    let mut p = Params::default();
    p.corpo.tipo = TipoCorpo::Serial;
    banca_com(&p)
}

fn banca_com(p: &Params) -> Banca {
    let corpo = CorpoFalso::default();
    let saude = Arc::new(Saude::default());
    let laco = Laco::new(
        corpo.clone(),
        p,
        Arc::new(Intencoes::default()),
        saude.clone(),
    );
    Banca {
        corpo,
        saude,
        laco,
        agora: Instant::now(),
    }
}

/// Um pacote de 163 bytes na ordem que nós assinamos, com um estado e um relógio dados.
fn pacote_nosso(mensagem: u8, seq: u16, estado: u32) -> Quadro {
    let mut d = vec![0u8; 163];
    d[1] = mensagem;
    d[130..134].copy_from_slice(&estado.to_le_bytes());
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
}

fn resposta(cmd: (u8, u8), codigo: u8) -> Quadro {
    Quadro {
        origem: 0x03,
        destino: 0x09,
        seq: 0,
        resposta: true,
        pede: PedeAck::Nada,
        cmd_set: cmd.0,
        cmd_id: cmd.1,
        dados: vec![codigo],
    }
}

impl Banca {
    fn corpo_manda(&self, q: &Quadro) {
        self.corpo.a_ler.lock().unwrap().extend(q.montar().unwrap());
    }

    fn ciclo(&mut self, ms: u64) {
        self.agora += Duration::from_millis(ms);
        self.laco.ciclo(self.agora, false);
    }

    fn saidos(&self, cmd: (u8, u8)) -> Vec<Quadro> {
        self.corpo
            .quadros()
            .into_iter()
            .filter(|q| (q.cmd_set, q.cmd_id) == cmd)
            .collect()
    }

    fn enlace(&self) -> Enlace {
        self.saude.amostra().expect("publicada").enlace
    }

    /// Passa a janela de observação.
    fn observar(&mut self, mensagens: &[u8]) {
        self.ciclo(10); // a janela conta do primeiro ciclo
        for (i, m) in mensagens.iter().enumerate() {
            self.corpo_manda(&pacote_nosso(*m, i as u16, 1));
            self.ciclo(10);
        }
        self.ciclo(3000);
    }

    /// Até receber, com as respostas de sucesso.
    fn ate_receber(&mut self) {
        self.observar(&[]);
        self.corpo_manda(&resposta(id::REGISTRAR, 0));
        self.ciclo(10);
        self.corpo_manda(&resposta(id::ASSINAR, 0));
        self.ciclo(10);
        self.corpo_manda(&pacote_nosso(1, 100, 1));
        self.ciclo(10);
        assert_eq!(self.enlace(), Enlace::Recebendo);
    }
}

/// A sequência da gravação da DJI: vê, cancela, registra e assina, nesta ordem.
#[test]
fn observa_cancela_o_que_viu_registra_e_assina() {
    let mut b = banca();
    b.observar(&[0, 1]);
    assert_eq!(b.enlace(), Enlace::Cancelando);
    let c = b.saidos(id::CANCELAR);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].dados, vec![0, 9, 0], "a menor primeiro");
    b.corpo_manda(&resposta(id::CANCELAR, 0));
    b.ciclo(10);
    assert_eq!(b.saidos(id::CANCELAR)[1].dados, vec![0, 9, 1]);
    assert!(
        b.saidos(id::REGISTRAR).is_empty(),
        "registrar só depois de cancelar tudo"
    );
    b.corpo_manda(&resposta(id::CANCELAR, 0x0c));
    b.ciclo(10);
    assert_eq!(
        b.saidos(id::REGISTRAR).len(),
        1,
        "mensagem que não existe mais não trava"
    );
    b.corpo_manda(&resposta(id::REGISTRAR, 0x50));
    b.ciclo(10);
    let a = b.saidos(id::ASSINAR);
    assert_eq!(a.len(), 1, "0x50, nó já existe, é o caso comum, sem reset");
    assert_eq!(
        &a[0].dados[..5],
        &[9, 1, 3, 0, 9],
        "mensagem 1, configuração 3, 9 dados"
    );
    b.corpo_manda(&resposta(id::ASSINAR, 0));
    b.ciclo(10);
    assert_eq!(b.enlace(), Enlace::Esperando);
    b.corpo_manda(&pacote_nosso(1, 200, 1));
    b.ciclo(10);
    assert_eq!(b.enlace(), Enlace::Recebendo);
}

/// A DJI responde a todos, com a mesma sequência, e desde o começo.
#[test]
fn responde_a_cada_pacote_com_a_mesma_sequencia() {
    let mut b = banca();
    b.ate_receber();
    b.corpo.limpar();
    for seq in [7, 8, 9] {
        b.corpo_manda(&pacote_nosso(1, seq, 1));
    }
    b.ciclo(10);
    let r = b.saidos(id::PACOTE);
    assert_eq!(r.iter().map(|q| q.seq).collect::<Vec<_>>(), vec![7, 8, 9]);
    assert!(
        r.iter()
            .all(|q| q.resposta && q.dados == vec![1] && q.destino == 0x03)
    );
    assert_eq!(r[0].montar().unwrap()[8], 0xa0, "atributo gravado");
}

#[test]
fn durante_a_observacao_tambem_responde_e_nao_decodifica() {
    let mut b = banca();
    b.corpo_manda(&pacote_nosso(1, 5, 1));
    b.ciclo(10);
    assert_eq!(b.saidos(id::PACOTE).len(), 1);
    let a = b.saude.amostra().unwrap();
    assert_eq!(
        a.pacotes, 0,
        "a ordem ainda não é conhecida: nada é adivinhado"
    );
    assert!(a.medidas.is_none());
}

/// Recusa não se repete: o código é a resposta.
#[test]
fn recusa_para_e_diz_o_codigo_sem_repetir() {
    for (passo, cmd, codigo) in [
        (Passo::Registrar, id::REGISTRAR, 0x02),
        (Passo::Assinar, id::ASSINAR, 0x23),
    ] {
        let mut b = banca();
        b.observar(&[]);
        if passo == Passo::Assinar {
            b.corpo_manda(&resposta(id::REGISTRAR, 0));
            b.ciclo(10);
        }
        b.corpo_manda(&resposta(cmd, codigo));
        b.ciclo(10);
        assert_eq!(b.enlace(), Enlace::Recusado { passo, codigo });
        let n = b.saidos(id::REGISTRAR).len() + b.saidos(id::ASSINAR).len();
        for _ in 0..50 {
            b.ciclo(1000);
        }
        assert_eq!(
            b.saidos(id::REGISTRAR).len() + b.saidos(id::ASSINAR).len(),
            n,
            "repetiu depois de recusa"
        );
        assert_eq!(b.enlace(), Enlace::Recusado { passo, codigo });
    }
}

/// Sem resposta: tentativas contadas, e para. Só recomeça com quadros chegando.
#[test]
fn sem_resposta_desiste_e_so_recomeca_com_quadros_chegando() {
    let mut b = banca();
    b.observar(&[]);
    for _ in 0..10 {
        b.ciclo(600);
    }
    assert_eq!(b.saidos(id::REGISTRAR).len(), 3);
    assert_eq!(
        b.enlace(),
        Enlace::SemResposta {
            passo: Passo::Registrar
        }
    );
    for _ in 0..40 {
        b.ciclo(1000);
    }
    assert_eq!(
        b.saidos(id::REGISTRAR).len(),
        3,
        "sem ninguém do outro lado, não martela"
    );
    b.corpo_manda(&pacote_nosso(0, 1, 1));
    b.ciclo(10);
    b.ciclo(10);
    assert_eq!(b.enlace(), Enlace::Observando);
}

/// A resposta se perdeu, a primeira valeu: o reenvio ouve "já existe", e isso não é recusa.
#[test]
fn reenvio_que_ouve_ja_existe_nao_e_recusa() {
    let mut b = banca();
    b.observar(&[]);
    b.corpo_manda(&resposta(id::REGISTRAR, 0));
    b.ciclo(10);
    b.ciclo(600);
    assert_eq!(b.saidos(id::ASSINAR).len(), 2);
    b.corpo_manda(&resposta(id::ASSINAR, 0x04));
    b.ciclo(10);
    assert_eq!(b.enlace(), Enlace::Esperando);
}

/// É o caso de recarregar a página: o corpo simulado volta sem assinatura.
#[test]
fn pacotes_parados_recomecam_a_assinatura() {
    let mut b = banca();
    b.ate_receber();
    b.ciclo(3100);
    assert_eq!(b.enlace(), Enlace::Observando);
    b.corpo.limpar();
    b.ciclo(3000);
    assert_eq!(b.saidos(id::REGISTRAR).len(), 1);
}

/// Contador não se perde com assinante lento, e cada subida conta uma vez, venha do pacote ou do aviso.
#[test]
fn eventos_do_chassi_contam_por_subida_sem_duplicar() {
    let mut b = banca();
    b.ate_receber();
    let c0 = b.saude.amostra().unwrap().contagem_chassi;
    const LEVANTADO: u32 = 1 << 4;
    for (seq, e) in [
        (1, LEVANTADO),
        (2, LEVANTADO),
        (3, 1),
        (4, LEVANTADO),
        (5, 1),
    ] {
        b.corpo_manda(&pacote_nosso(1, seq, e));
    }
    b.ciclo(10);
    let a = b.saude.amostra().unwrap();
    assert_eq!(
        a.contagem_chassi[4] - c0[4],
        2,
        "levantado subiu duas vezes"
    );
    // O aviso e o pacote seguinte trazem o mesmo valor: 33 de 33 nas gravações.
    let aviso = Quadro {
        origem: 0x03,
        destino: 0x09,
        seq: 0,
        resposta: false,
        pede: PedeAck::Nada,
        cmd_set: 0x3f,
        cmd_id: 0x29,
        dados: (1u32 << 5).to_le_bytes().to_vec(),
    };
    b.corpo_manda(&aviso);
    b.corpo_manda(&pacote_nosso(1, 6, 1 << 5));
    b.ciclo(10);
    assert_eq!(b.saude.amostra().unwrap().contagem_chassi[5] - c0[5], 1);
}

/// Cada placa conta o dela, com tipo, microfone e aceleração, e placa fora da faixa não conta.
#[test]
fn toque_em_cada_placa_conta_na_placa_certa() {
    let mut b = banca();
    for placa in 1..=6u8 {
        for _ in 0..placa {
            let q = Quadro {
                origem: placa * 32 + 24,
                destino: 0x09,
                seq: 0,
                resposta: false,
                pede: PedeAck::Nada,
                cmd_set: 0x3f,
                cmd_id: 0x02,
                dados: vec![placa << 4 | 1, 0x23, 0x01, 0x22, 0],
            };
            b.corpo_manda(&q);
        }
    }
    b.corpo_manda(&Quadro {
        origem: 0x58,
        destino: 0x09,
        seq: 0,
        resposta: false,
        pede: PedeAck::Nada,
        cmd_set: 0x3f,
        cmd_id: 0x02,
        dados: vec![7 << 4, 0, 0, 0, 0],
    });
    b.ciclo(10);
    let a = b.saude.amostra().unwrap();
    for (i, p) in a.placas.iter().enumerate() {
        assert_eq!(p.toques, i as u64 + 1, "placa {}", i + 1);
        assert_eq!((p.tipo, p.microfone, p.aceleracao), (1, 0x0123, 0x22));
    }
    assert_eq!(a.ignorados, 1);
}

#[test]
fn infravermelho_conta_por_lado() {
    let mut b = banca();
    let ir = |bits| Quadro {
        origem: 0xd8,
        destino: 0x09,
        seq: 0,
        resposta: false,
        pede: PedeAck::Nada,
        cmd_set: 0x3f,
        cmd_id: 0x10,
        dados: vec![0, bits, 0],
    };
    b.corpo_manda(&ir(1));
    b.corpo_manda(&ir(3));
    b.ciclo(10);
    assert_eq!(b.saude.amostra().unwrap().infravermelho, [2, 1]);
}

/// Corpo que não recebe a telemetria não é corpo recusando comando: é o simulador sem página.
#[test]
fn erro_do_barramento_nao_conta_como_erro_de_comando() {
    use std::sync::atomic::Ordering;
    let mut b = banca();
    b.corpo.falhar.store(true, Ordering::Relaxed);
    b.observar(&[]);
    assert_eq!(b.saude.erros_seguidos.load(Ordering::Relaxed), 0);
    assert!(b.saude.erros_barramento.load(Ordering::Relaxed) > 0);
}

/// Quadro truncado custa ele, e não derruba nada.
#[test]
fn quadro_curto_custa_so_ele() {
    let mut b = banca();
    for (s, c) in [(0x3f, 0x29), (0x3f, 0x02), (0x3f, 0x10), (0x3f, 0x76)] {
        b.corpo_manda(&Quadro {
            origem: 3,
            destino: 9,
            seq: 0,
            resposta: false,
            pede: PedeAck::Nada,
            cmd_set: s,
            cmd_id: c,
            dados: vec![1],
        });
    }
    b.ciclo(10);
    assert_eq!(b.saude.amostra().unwrap().ignorados, 4);
}

/// O corpo falso não devolve nada: não se assina, e nada sai sem armar.
#[test]
fn corpo_falso_nao_assina() {
    let mut b = banca_com(&Params::default());
    for _ in 0..20 {
        b.ciclo(1000);
    }
    assert!(b.corpo.quadros().is_empty(), "{:?}", b.corpo.quadros());
    assert_eq!(b.enlace(), Enlace::Desligado);
}

/// Os eixos do contrato: y à esquerda, z para cima, rumo à esquerda, e rodas com sinal de andar para a frente.
#[test]
fn a_conversao_inverte_o_que_tem_de_inverter() {
    let q = Quadro::ler(PARADO).unwrap();
    let mut d = q.dados.clone();
    let l = Layout::da_assinatura();
    let p0 = Pacote::ler(&d, &l).unwrap();
    assert_eq!(p0.mensagem, d[1]);
    let f = |d: &mut Vec<u8>, pos: usize, v: f32| d[pos..pos + 4].copy_from_slice(&v.to_le_bytes());
    f(&mut d, 94, 1.0); // posição x
    f(&mut d, 98, 2.0); // posição y, à direita
    f(&mut d, 114, -1.0); // aceleração z, para baixo
    f(&mut d, 126, 0.5); // giro em z, à direita
    f(&mut d, 134, 30.0); // rumo, à direita
    for (i, v) in [100i16, -100, -100, 100].iter().enumerate() {
        let pos = 58 + i * 2;
        d[pos..pos + 2].copy_from_slice(&v.to_le_bytes());
    }
    let m = Medidas::de(&Pacote::ler(&d, &l).unwrap());
    assert_eq!(m.posicao[0], 1.0);
    assert_eq!(m.posicao[1], -2.0, "y à esquerda");
    assert_eq!(m.aceleracao[2], 1.0, "z para cima");
    assert_eq!(m.gravidade()[2], -1.0, "como o contrato pede em pé");
    assert_eq!(m.giro[2], -0.5);
    assert_eq!(m.rumo, -30.0, "à esquerda é positivo");
    assert_eq!(
        m.rodas_rpm, [100.0; 4],
        "as quatro para a frente, desfeita a inversão do fio"
    );
    let w = m.quaternion();
    let norma = w.iter().map(|v| v * v).sum::<f64>();
    assert!((norma - 1.0).abs() < 1e-9);
    assert!((w[3] - (-15f64).to_radians().sin()).abs() < 0.02, "{w:?}");
}
