//! O comportamento pelos quadros que chegam ao corpo, que é o que o robô vê.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use rms1_control::cinematica::rpm_de_twist;
use rms1_control::intencoes::Modo;
use rms1_control::{CorpoFalso, Intencoes, Laco, Saude};
use rms1_proto::Quadro;
use rms1_proto::comando::id;
use rms1d_params::Params;

struct Banca {
    corpo: CorpoFalso,
    intencoes: Arc<Intencoes>,
    saude: Arc<Saude>,
    laco: Laco<CorpoFalso>,
    agora: Instant,
}

fn banca_com(p: &Params) -> Banca {
    let corpo = CorpoFalso::default();
    let intencoes = Arc::new(Intencoes::default());
    let saude = Arc::new(Saude::default());
    let laco = Laco::new(corpo.clone(), p, intencoes.clone(), saude.clone());
    Banca {
        corpo,
        intencoes,
        saude,
        laco,
        agora: Instant::now(),
    }
}

fn banca() -> Banca {
    banca_com(&Params::default())
}

impl Banca {
    fn ciclo(&mut self) -> rms1_control::Estado {
        self.agora += Duration::from_millis(20);
        self.laco.ciclo(self.agora, true).expect("pedido estado")
    }

    fn armar(&mut self) {
        self.intencoes.pedir_armar(true);
        self.ciclo();
        self.corpo.limpar();
    }

    /// Pede agora, e faz o tempo do relógio do teste andar junto com o carimbo real.
    fn pedir_e_ciclar(&mut self, twist: [f64; 3]) -> rms1_control::Estado {
        self.intencoes.pedir_twist(twist);
        self.agora = Instant::now();
        self.laco.ciclo(self.agora, true).expect("pedido estado")
    }

    fn de(&self, cmd: (u8, u8)) -> Vec<Quadro> {
        self.corpo
            .quadros()
            .into_iter()
            .filter(|q| (q.cmd_set, q.cmd_id) == cmd)
            .collect()
    }

    fn ultimas_rodas(&self) -> [i16; 4] {
        let q = self.de(id::RODAS).pop().expect("há rodas");
        let i = |p: usize| i16::from_le_bytes([q.dados[p], q.dados[p + 1]]);
        // Desfaz a inversão da roda 2 e da 3 do fio.
        [i(0), -i(2), -i(4), i(6)]
    }
}

/// Ligar o processo não move nada: nem um quadro, por quanto tempo for, mesmo com pedido chegando.
#[test]
fn sem_armar_nenhum_quadro_chega_ao_corpo() {
    let mut b = banca();
    for _ in 0..100 {
        b.ciclo();
    }
    let e = b.pedir_e_ciclar([1.0, 0.0, 0.0]);
    b.intencoes.pedir_torre(0.5, 0.1);
    b.ciclo();
    assert!(b.corpo.quadros().is_empty(), "{:?}", b.corpo.quadros());
    assert_eq!(e.aplicado, [0.0; 3]);
    assert_eq!(e.limitado_por, vec!["disarmed"], "o cliente sabe por quê");
}

/// Armar liga o chassi e manda a permissão a cada ciclo, com as rodas paradas se nada foi pedido.
#[test]
fn armar_envia_modo_e_a_permissao_a_cada_ciclo() {
    let mut b = banca();
    b.intencoes.pedir_armar(true);
    b.ciclo();
    assert_eq!(b.de(id::MODO_CHASSI).len(), 5);
    assert_eq!(b.de(id::MODO_CHASSI)[0].dados, vec![1]);
    assert_eq!(b.de(id::MODO_TORRE)[0].dados, vec![0, 0]);
    let n = b.de(id::PERMISSAO).len();
    for _ in 0..10 {
        b.ciclo();
    }
    assert_eq!(b.de(id::PERMISSAO).len(), n + 10);
    assert_eq!(b.ultimas_rodas(), [0; 4], "nada pedido, nada anda");
    assert!(
        b.de(id::TORRE_ANGULOS).is_empty(),
        "armar não leva a torre a lugar nenhum"
    );
}

/// As rodas pedidas, com a cinemática, e sinais da biblioteca.
#[test]
fn robot_move_vira_rodas_com_sinais_da_biblioteca() {
    let mut b = banca();
    b.armar();
    let p = Params::default();
    for twist in [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.5, -0.3, 0.7],
    ] {
        let e = b.pedir_e_ciclar(twist);
        let esperado = rpm_de_twist(twist, p.geometria.raio_roda_m, p.geometria.meia_soma_m)
            .map(|v| v.round() as i16);
        assert_eq!(b.ultimas_rodas(), esperado, "{twist:?}");
        assert_eq!(e.aplicado, twist);
        assert!(e.limitado_por.is_empty());
    }
}

/// Andar para a esquerda, no fio: a roda 2 e a 3 vão invertidas (`rm_ctrl.py:394`).
#[test]
fn para_a_frente_o_fio_tem_a_roda_2_e_a_3_negativas() {
    let mut b = banca();
    b.armar();
    b.pedir_e_ciclar([1.0, 0.0, 0.0]);
    let q = b.de(id::RODAS).pop().unwrap();
    let i = |p: usize| i16::from_le_bytes([q.dados[p], q.dados[p + 1]]);
    assert!(
        i(0) > 0 && i(2) < 0 && i(4) < 0 && i(6) > 0,
        "{:?}",
        q.dados
    );
}

/// Sem comando dentro do prazo, as rodas zeram, e o prazo vem de `rms1d.toml`.
#[test]
fn sem_comando_alem_do_prazo_as_rodas_zeram() {
    let mut p = Params::default();
    p.seguranca.prazo_ms = 50;
    let mut b = banca_com(&p);
    b.armar();
    b.pedir_e_ciclar([1.0, 0.0, 0.0]);
    assert_ne!(b.ultimas_rodas(), [0; 4]);
    std::thread::sleep(Duration::from_millis(80));
    b.agora = Instant::now();
    let e = b.laco.ciclo(b.agora, true).unwrap();
    assert_eq!(b.ultimas_rodas(), [0; 4]);
    assert_eq!(e.limitado_por, vec!["deadman"]);
    assert_eq!(e.pedido, [1.0, 0.0, 0.0], "o pedido segue visível");
    assert_eq!(e.aplicado, [0.0; 3]);
}

#[test]
fn o_prazo_padrao_e_500_ms() {
    assert_eq!(Params::default().seguranca.prazo_ms, 500);
}

/// A velocidade é limitada à faixa, e reportada.
#[test]
fn velocidade_acima_da_faixa_e_limitada_e_reportada() {
    let mut b = banca();
    b.armar();
    let e = b.pedir_e_ciclar([10.0, 0.0, 0.0]);
    assert_eq!(e.aplicado, [3.5, 0.0, 0.0]);
    assert!(e.limitado_por.contains(&"max_velocity"));
}

/// Rodas acima de 1000 rpm encolhem juntas: cortar só uma mudaria o rumo.
#[test]
fn rpm_acima_do_limite_encolhe_as_quatro_juntas() {
    let mut b = banca();
    b.armar();
    let e = b.pedir_e_ciclar([3.5, 3.5, 0.0]);
    let r = b.ultimas_rodas();
    assert!(r.iter().all(|v| v.abs() <= 1000), "{r:?}");
    assert_eq!(r[0], 1000);
    assert_eq!(r[1], 0);
    assert!(e.limitado_por.contains(&"max_rpm"));
}

/// `NaN` é recusado, nunca limitado: limitar daria um valor plausível na ponta.
#[test]
fn valor_nao_finito_e_recusado_e_o_robo_para() {
    let mut b = banca();
    b.armar();
    let e = b.pedir_e_ciclar([f64::NAN, 0.0, 0.0]);
    assert_eq!(b.ultimas_rodas(), [0; 4]);
    assert_eq!(e.limitado_por, vec!["not_finite"]);
    b.corpo.limpar();
    b.intencoes.pedir_torre(f64::INFINITY, 0.0);
    let e = b.ciclo();
    assert!(b.de(id::TORRE_ANGULOS).is_empty());
    assert!(e.limitado_por.contains(&"not_finite"));
}

fn i16_em(q: &Quadro, p: usize) -> i16 {
    i16::from_le_bytes([q.dados[p], q.dados[p + 1]])
}

/// Contrato: yaw à esquerda e pitch para baixo. DJI: à direita e para cima. Viu-se no robô, em
/// 2026-10-30, a torre olhar para o lado errado por isso.
#[test]
fn a_torre_inverte_os_dois_sinais_para_a_dji() {
    let mut b = banca();
    b.armar();
    b.intencoes
        .pedir_torre(10f64.to_radians(), 5f64.to_radians());
    b.ciclo();
    let q = b.de(id::TORRE_ANGULOS).pop().unwrap();
    assert_eq!((i16_em(&q, 3), i16_em(&q, 7)), (-100, -50));
    assert_eq!(q.dados[2], (4 << 3) | 0x05, "coordenada 4, os dois eixos");
}

/// Limites do firmware: ±250° e −20° a 35°.
#[test]
fn a_torre_fica_nos_limites_do_firmware() {
    let mut b = banca();
    b.armar();
    b.intencoes
        .pedir_torre(-300f64.to_radians(), -90f64.to_radians());
    b.ciclo();
    let q = b.de(id::TORRE_ANGULOS).pop().unwrap();
    assert_eq!((i16_em(&q, 3), i16_em(&q, 7)), (2500, 350));
    b.intencoes
        .pedir_torre(300f64.to_radians(), 90f64.to_radians());
    b.ciclo();
    let q = b.de(id::TORRE_ANGULOS).pop().unwrap();
    assert_eq!((i16_em(&q, 3), i16_em(&q, 7)), (-2500, -200));
}

/// Torre segue chassi: a DJI desliga o eixo de giro (`angle_ctrl`), e só o pitch é nosso.
#[test]
fn com_a_torre_seguindo_o_giro_nao_e_enviado() {
    let mut b = banca();
    b.armar();
    b.intencoes.pedir_modo(Modo::TorreSegueChassi);
    b.intencoes.pedir_torre(1.0, 0.1);
    b.ciclo();
    assert_eq!(b.de(id::MODO_TORRE).pop().unwrap().dados, vec![2, 0]);
    let q = b.de(id::TORRE_ANGULOS).pop().unwrap();
    assert_eq!(q.dados[2] & 0x07, 0x04, "só o eixo de pitch");
    // E as rodas continuam nossas, com o giro pedido.
    b.pedir_e_ciclar([0.0, 0.0, 1.0]);
    assert_ne!(b.ultimas_rodas(), [0; 4]);
    assert!(b.de(id::SEGUIR).is_empty());
}

/// Chassi segue torre: `0x3f/0x22`, e o giro pedido vira taxa da torre, que o chassi persegue.
#[test]
fn com_o_chassi_seguindo_o_movimento_vai_em_seguir_e_o_giro_na_torre() {
    let mut b = banca();
    b.armar();
    b.intencoes.pedir_modo(Modo::ChassiSegueTorre);
    b.pedir_e_ciclar([0.5, 0.25, 1.0]);
    assert_eq!(b.de(id::MODO_TORRE).pop().unwrap().dados, vec![0, 0]);
    assert!(b.de(id::RODAS).is_empty(), "as rodas são do controlador");
    let q = b.de(id::SEGUIR).pop().unwrap();
    let f = |p: usize| f32::from_le_bytes(q.dados[p..p + 4].try_into().unwrap());
    assert_eq!(
        (f(0), f(4), f(8)),
        (0.5, -0.25, 0.0),
        "contrato com y à esquerda, DJI com y à direita (`_move_ctrl` em rm_ctrl.py)"
    );
    let t = b.de(id::TORRE_TAXA).pop().unwrap();
    assert_eq!(
        i16_em(&t, 0),
        -573,
        "1 rad/s à esquerda, em décimos de grau para a direita"
    );
    assert_eq!(t.dados[6], 0xdc);
    // Parou de girar: uma taxa zero, com controle zero, e uma vez só.
    b.corpo.limpar();
    b.pedir_e_ciclar([0.5, 0.0, 0.0]);
    b.pedir_e_ciclar([0.5, 0.0, 0.0]);
    let t = b.de(id::TORRE_TAXA);
    assert_eq!(t.len(), 1);
    assert_eq!((i16_em(&t[0], 0), t[0].dados[6]), (0, 0x00));
}

/// Um alvo em relação ao chassi reenviado faria o chassi nunca parar: uma vez por mudança.
#[test]
fn com_o_chassi_seguindo_o_alvo_da_torre_vai_uma_vez_so() {
    let mut b = banca();
    b.armar();
    b.intencoes.pedir_modo(Modo::ChassiSegueTorre);
    b.intencoes.pedir_torre(0.3, 0.0);
    for _ in 0..30 {
        b.ciclo();
    }
    assert_eq!(b.de(id::TORRE_ANGULOS).len(), 1);
}

/// No modo livre o alvo é reenviado, como a prova de 2026-10-29 fez.
#[test]
fn no_modo_livre_o_alvo_da_torre_e_reenviado() {
    let mut b = banca();
    b.armar();
    b.intencoes.pedir_torre(0.3, 0.0);
    for _ in 0..30 {
        b.ciclo();
    }
    assert_eq!(b.de(id::TORRE_ANGULOS).len(), 6, "600 ms a 100 ms");
}

/// Desarmar zera as rodas e corta a permissão, que é o que desarma o chassi.
#[test]
fn desarmar_zera_as_rodas_e_corta_a_permissao() {
    let mut b = banca();
    b.armar();
    b.pedir_e_ciclar([1.0, 0.0, 0.0]);
    b.intencoes.pedir_armar(false);
    b.ciclo();
    assert_eq!(b.ultimas_rodas(), [0; 4]);
    b.corpo.limpar();
    for _ in 0..10 {
        b.ciclo();
    }
    assert!(b.corpo.quadros().is_empty());
    assert!(!b.saude.armado.load(Ordering::Relaxed));
}

/// Seguindo a torre, rodas não valem: parar é seguir a zero, e a torre que girava para (`rm_ctrl.py:1102` e `:1486`).
#[test]
fn desarmar_seguindo_a_torre_zera_o_seguir_e_a_taxa() {
    let mut b = banca();
    b.armar();
    b.intencoes.pedir_modo(Modo::ChassiSegueTorre);
    b.pedir_e_ciclar([0.5, 0.25, 1.0]);
    b.corpo.limpar();
    b.intencoes.pedir_armar(false);
    b.ciclo();
    assert!(b.de(id::RODAS).is_empty(), "rodas não valem neste modo");
    let s = b.de(id::SEGUIR).pop().expect("seguir a zero");
    assert_eq!(s.dados, vec![0; 12]);
    let t = b.de(id::TORRE_TAXA).pop().expect("a taxa a zero");
    assert_eq!((i16_em(&t, 0), t.dados[6]), (0, 0x00));
}

/// Desarmar nem a torre que não girava recebe taxa.
#[test]
fn desarmar_sem_taxa_ligada_nao_manda_taxa() {
    let mut b = banca();
    b.armar();
    b.pedir_e_ciclar([0.5, 0.0, 0.0]);
    b.corpo.limpar();
    b.intencoes.pedir_armar(false);
    b.ciclo();
    assert!(b.de(id::TORRE_TAXA).is_empty());
}

/// O prazo vale nos três modos: seguindo a torre, a velocidade e o giro zeram.
#[test]
fn sem_comando_alem_do_prazo_seguindo_a_torre_tudo_zera() {
    let mut p = Params::default();
    p.seguranca.prazo_ms = 50;
    let mut b = banca_com(&p);
    b.armar();
    b.intencoes.pedir_modo(Modo::ChassiSegueTorre);
    b.pedir_e_ciclar([0.5, 0.25, 1.0]);
    b.corpo.limpar();
    std::thread::sleep(Duration::from_millis(80));
    b.agora = Instant::now();
    let e = b.laco.ciclo(b.agora, true).unwrap();
    assert_eq!(b.de(id::SEGUIR).pop().unwrap().dados, vec![0; 12]);
    let t = b.de(id::TORRE_TAXA).pop().expect("a taxa a zero");
    assert_eq!((i16_em(&t, 0), t.dados[6]), (0, 0x00));
    assert_eq!(e.limitado_por, vec!["deadman"]);
}

/// Encerrar o processo armado para o robô, em qualquer modo.
#[test]
fn encerrar_armado_para_como_desarmar() {
    for (modo, cmd) in [
        (Modo::Livre, id::RODAS),
        (Modo::ChassiSegueTorre, id::SEGUIR),
    ] {
        let mut b = banca();
        b.armar();
        b.intencoes.pedir_modo(modo);
        b.pedir_e_ciclar([0.5, 0.0, 0.0]);
        b.corpo.limpar();
        b.laco.encerrar();
        let q = b.de(cmd);
        assert_eq!(q.len(), 10, "{modo:?}");
        assert!(q.iter().all(|x| x.dados.iter().all(|v| *v == 0)), "{q:?}");
    }
}

/// O identificador de tarefa troca a cada envio, nunca é 0, e dá a volta (`rm_module.py:429`).
#[test]
fn a_tarefa_da_torre_troca_a_cada_envio_e_nunca_e_zero() {
    let mut b = banca();
    b.armar();
    let mut vistas = Vec::new();
    for i in 0..300u32 {
        // De 0,002 rad em 0,002 rad: mais de um décimo de grau, então cada alvo é novo e sai neste ciclo.
        b.intencoes.pedir_torre(f64::from(i) * 0.002, 0.0);
        b.ciclo();
        vistas.push(b.de(id::TORRE_ANGULOS).pop().unwrap().dados[0]);
    }
    assert_eq!(&vistas[..3], &[1, 2, 3]);
    assert!(vistas.iter().all(|t| *t != 0 && *t != 255), "{vistas:?}");
    assert_eq!(vistas[253], 254);
    assert_eq!(vistas[254], 1, "dá a volta");
}

/// O estado diz o que as rodas receberam, e não o que foi pedido antes de encolher.
#[test]
fn o_aplicado_encolhe_junto_com_as_rodas() {
    let mut b = banca();
    b.armar();
    let e = b.pedir_e_ciclar([3.5, 3.5, 0.0]);
    let p = Params::default();
    let de_verdade = rpm_de_twist(e.aplicado, p.geometria.raio_roda_m, p.geometria.meia_soma_m);
    let maior = de_verdade.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
    assert!(
        (maior - 1000.0).abs() < 1e-6,
        "{:?} dá {maior} rpm",
        e.aplicado
    );
    assert!(
        (e.aplicado[0] - e.aplicado[1]).abs() < 1e-9,
        "encolhe sem mudar o rumo"
    );
}

/// O estado da torre é o alvo depois do limite, e o limite é dito.
#[test]
fn o_estado_da_torre_e_o_alvo_limitado_e_diz_que_limitou() {
    let mut b = banca();
    b.armar();
    assert_eq!(b.ciclo().torre, None, "ninguém pediu");
    b.intencoes.pedir_torre(6.0, -2.0);
    let e = b.ciclo();
    let [yaw, pitch] = e.torre.unwrap();
    assert!((yaw - 250f64.to_radians()).abs() < 1e-3, "{yaw}");
    assert!((pitch + 35f64.to_radians()).abs() < 1e-3, "{pitch}");
    assert!(
        e.limitado_por.contains(&"gimbal_range"),
        "{:?}",
        e.limitado_por
    );
    b.intencoes.pedir_torre(0.1, 0.1);
    let e = b.ciclo();
    assert!(e.limitado_por.is_empty(), "{:?}", e.limitado_por);
}

/// Com a torre seguindo o chassi, o yaw pedido não é nosso: não aparece como aplicado, nem como limite.
#[test]
fn com_a_torre_seguindo_o_estado_nao_mostra_yaw_pedido() {
    let mut b = banca();
    b.armar();
    b.intencoes.pedir_modo(Modo::TorreSegueChassi);
    b.intencoes.pedir_torre(6.0, 0.1);
    let e = b.ciclo();
    let [yaw, pitch] = e.torre.unwrap();
    assert_eq!(yaw, 0.0);
    assert!((pitch - 0.1).abs() < 2e-3, "{pitch}");
    assert!(e.limitado_por.is_empty(), "{:?}", e.limitado_por);
}

/// O último pedido de energia vence, e é lido uma vez por ciclo.
#[test]
fn o_ultimo_pedido_de_energia_vence() {
    let mut b = banca();
    b.intencoes.pedir_armar(true);
    b.intencoes.pedir_armar(false);
    b.ciclo();
    assert!(b.corpo.quadros().is_empty());
    assert!(!b.saude.armado.load(Ordering::Relaxed));
}

/// Nenhum caminho envia o reset de nó de telemetria, que deixou o barramento morto até desligar.
#[test]
fn nenhum_caminho_envia_reset_de_no() {
    let mut b = banca();
    b.armar();
    for m in Modo::TODOS {
        b.intencoes.pedir_modo(m);
        b.intencoes.pedir_torre(0.2, 0.1);
        b.pedir_e_ciclar([0.5, 0.1, 0.5]);
    }
    b.intencoes.pedir_armar(false);
    b.ciclo();
    assert!(
        b.corpo.quadros().iter().all(|q| q.cmd_set != 0x48),
        "{:?}",
        b.corpo.quadros()
    );
}

/// Corpo que falha não derruba o ciclo, e aparece na saúde como problema do corpo.
#[test]
fn corpo_que_falha_aparece_na_saude_como_degradado() {
    let mut b = banca();
    b.armar();
    b.corpo.falhar.store(true, Ordering::Relaxed);
    b.ciclo();
    let v = b
        .saude
        .veredito(Instant::now(), 0.0, Duration::from_secs(5));
    assert!(!v.saudavel && v.degradado, "{v:?}");
    b.corpo.falhar.store(false, Ordering::Relaxed);
    b.ciclo();
    let v = b
        .saude
        .veredito(Instant::now(), 0.0, Duration::from_secs(5));
    assert!(v.saudavel, "{v:?}");
}

/// Laço travado se denuncia: a saúde é calculada dos atômicos, sem perguntar ao laço.
#[test]
fn laco_parado_se_denuncia_pela_idade() {
    let saude = Saude::default();
    saude.carimbar_ciclo(Instant::now());
    std::thread::sleep(Duration::from_millis(30));
    let v = saude.veredito(Instant::now(), 0.0, Duration::from_millis(10));
    assert!(!v.saudavel && !v.degradado, "{v:?}");
    assert!(v.motivo.unwrap().contains("parou"));
}

/// Sem assinante, nada é montado.
#[test]
fn sem_assinante_o_estado_nao_e_montado() {
    let mut b = banca();
    assert!(b.laco.ciclo(Instant::now(), false).is_none());
}

/// Um cliente martelando pedidos não atrasa o laço: não há trava entre os dois. Cliente travado de verdade,
/// no socket, é `cliente_que_nao_le_nao_atrasa_o_laco`, em `rms1d/tests/processo.rs`.
#[test]
fn pedidos_em_rajada_nao_atrasam_o_laco() {
    let mut b = banca();
    b.armar();
    let i = b.intencoes.clone();
    let fim = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let f = fim.clone();
    let martelo = std::thread::spawn(move || {
        while !f.load(Ordering::Relaxed) {
            i.pedir_twist([0.1, 0.0, 0.0]);
            i.pedir_torre(0.1, 0.1);
        }
    });
    let mut pior = Duration::ZERO;
    for _ in 0..500 {
        let t = Instant::now();
        b.laco.ciclo(t, true);
        pior = pior.max(t.elapsed());
    }
    fim.store(true, Ordering::Relaxed);
    martelo.join().unwrap();
    assert!(
        pior < Duration::from_millis(20),
        "ciclo levou {pior:?}, mais que o período"
    );
}
