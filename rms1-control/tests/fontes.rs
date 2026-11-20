//! Quem manda no corpo: a tabela de fontes, pelo que o laço aplica e pelos quadros que chegam ao corpo.
//!
//! O carimbo de um pedido é o relógio de verdade, e o laço recebe o `agora` de quem chama. Para provar um prazo sem
//! depender de quanto o teste dormiu, cada pedido fica entre dois instantes medidos (`antes` e `depois`): em
//! `antes + prazo` ele com certeza está vivo, e em `depois + prazo + 1 ms` com certeza venceu.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rms1_control::{CorpoFalso, Estado, FonteId, Intencoes, Laco, Saude};
use rms1_proto::Quadro;
use rms1_proto::comando::id;
use rms1d_params::{Canal, FONTE_SEM_NOME, Fonte, Params};

struct Banca {
    corpo: CorpoFalso,
    i: Arc<Intencoes>,
    laco: Laco<CorpoFalso>,
    piloto: FonteId,
    agente: FonteId,
    cerebro: FonteId,
    prazo: Duration,
}

/// A tabela de produção, a de `Params::default()`: piloto, agente, cérebro e a fonte sem nome.
fn banca() -> Banca {
    banca_com(Params::default())
}

fn banca_com(p: Params) -> Banca {
    p.validar().unwrap();
    let prazo = Duration::from_millis(p.seguranca.prazo_ms);
    let corpo = CorpoFalso::default();
    let i = Arc::new(Intencoes::com_fontes(&p.fontes, prazo));
    let laco = Laco::new(corpo.clone(), &p, i.clone(), Arc::new(Saude::default()));
    let mut b = Banca {
        piloto: i.fonte(Some("piloto")),
        agente: i.fonte(Some("agente")),
        cerebro: i.fonte(Some("cerebro")),
        corpo,
        i,
        laco,
        prazo,
    };
    b.i.pedir_armar(true);
    b.em(Instant::now());
    b.corpo.limpar();
    b
}

impl Banca {
    fn em(&mut self, agora: Instant) -> Estado {
        self.laco.ciclo(agora, true).expect("pedido estado")
    }

    fn ja(&mut self) -> Estado {
        self.em(Instant::now())
    }

    fn de(&self, cmd: (u8, u8)) -> Vec<Quadro> {
        self.corpo
            .quadros()
            .into_iter()
            .filter(|q| (q.cmd_set, q.cmd_id) == cmd)
            .collect()
    }
}

fn nome(d: &Option<Arc<str>>) -> Option<&str> {
    d.as_deref()
}

/// Um pedido entre dois instantes medidos.
fn medido(pedir: impl FnOnce() -> bool) -> (Instant, Instant) {
    let antes = Instant::now();
    assert!(pedir(), "o pedido tinha de ser guardado");
    (antes, Instant::now())
}

const FRENTE: [f64; 3] = [0.3, 0.0, 0.0];
const GIRO: [f64; 3] = [0.0, 0.0, 0.5];
const MS: Duration = Duration::from_millis(1);

/// O nome dito no `hello` escolhe a linha; nome que a tabela não tem, ou nenhum, é a fonte sem nome.
#[test]
fn o_nome_do_hello_escolhe_a_fonte() {
    let b = banca();
    assert_eq!(&*b.i.nome(b.piloto), "piloto");
    assert_eq!(&*b.i.nome(b.agente), "agente");
    assert_eq!(&*b.i.nome(b.cerebro), "cerebro");
    assert_ne!(b.piloto, FonteId::SEM_NOME);
    for sem in [None, Some("padd"), Some(""), Some(FONTE_SEM_NOME)] {
        assert_eq!(b.i.fonte(sem), FonteId::SEM_NOME, "{sem:?}");
    }
    assert_eq!(&*b.i.nome(FonteId::SEM_NOME), FONTE_SEM_NOME);
    // Sem tabela, só há a fonte sem nome, e ela pode tudo: é o quadro único de antes.
    let so = Intencoes::default();
    assert_eq!(so.fonte(Some("piloto")), FonteId::SEM_NOME);
    assert!(Canal::TODOS.iter().all(|c| so.pode(FonteId::SEM_NOME, *c)));
}

/// A prioridade decide, e não a ordem de chegada: o agente pede por último, e quem anda é o piloto.
#[test]
fn o_piloto_vence_o_agente_mesmo_pedindo_antes() {
    let mut b = banca();
    assert!(b.i.pedir_twist(b.agente, FRENTE));
    let e = b.ja();
    assert_eq!(e.aplicado, FRENTE, "sozinho, o agente anda");
    assert_eq!(nome(&e.dono_rodas), Some("agente"));
    assert!(b.i.pedir_twist(b.piloto, GIRO));
    assert!(
        b.i.pedir_twist(b.agente, FRENTE),
        "o do agente fica guardado"
    );
    let e = b.ja();
    assert_eq!(
        e.aplicado, GIRO,
        "o aplicado é o do agente com o piloto pedindo"
    );
    assert_eq!(e.pedido, GIRO);
    assert_eq!(nome(&e.dono_rodas), Some("piloto"));
    assert_eq!(nome(&e.fonte_do_pedido), Some("piloto"));
    assert_eq!(
        b.i.quem_supera(b.agente, Canal::Rodas, Instant::now()),
        Some(b.piloto)
    );
    assert_eq!(
        b.i.quem_supera(b.piloto, Canal::Rodas, Instant::now()),
        None
    );
}

/// Aceite 5: o piloto para de pedir, com o agente renovando. As rodas voltam ao agente depois do prazo do piloto, e não
/// antes.
#[test]
fn as_rodas_voltam_ao_agente_depois_do_prazo_do_piloto() {
    let mut b = banca();
    assert_eq!(b.prazo, Duration::from_millis(500), "o prazo de produção");
    let (antes, depois) = medido(|| b.i.pedir_twist(b.piloto, GIRO));
    // O agente renova bem depois: quando o do piloto vencer, o dele ainda vale.
    std::thread::sleep(Duration::from_millis(150));
    assert!(b.i.pedir_twist(b.agente, FRENTE));
    let e = b.em(antes + b.prazo);
    assert_eq!(e.aplicado, GIRO, "as rodas voltaram antes do prazo");
    assert_eq!(nome(&e.dono_rodas), Some("piloto"));
    let e = b.em(depois + b.prazo + MS);
    assert_eq!(e.aplicado, FRENTE, "as rodas não voltam ao agente");
    assert_eq!(nome(&e.dono_rodas), Some("agente"));
    assert_eq!(nome(&e.fonte_do_pedido), Some("agente"));
    assert!(e.limitado_por.is_empty(), "{:?}", e.limitado_por);
}

/// Sem pedido vivo de ninguém, o estado mostra o pedido mais recente e diz que o prazo passou; e ninguém tem as rodas.
#[test]
fn vencido_de_todos_o_estado_diz_o_prazo_e_ninguem_tem_as_rodas() {
    let mut b = banca();
    assert!(b.i.pedir_twist(b.piloto, GIRO));
    std::thread::sleep(Duration::from_millis(20));
    let (_, depois) = medido(|| b.i.pedir_twist(b.agente, FRENTE));
    let e = b.em(depois + b.prazo + MS);
    assert_eq!(e.aplicado, [0.0; 3]);
    assert_eq!(e.pedido, FRENTE, "o mais recente");
    assert_eq!(e.limitado_por, vec!["deadman"]);
    assert_eq!(nome(&e.dono_rodas), None);
    assert_eq!(nome(&e.fonte_do_pedido), Some("agente"));
}

/// O prazo próprio de uma fonte vale para ela, na escolha e na segurança; as outras seguem com o de `[seguranca]`.
#[test]
fn o_prazo_proprio_da_fonte_vale_para_ela() {
    let mut p = Params::default();
    let curto = Duration::from_millis(120);
    p.fontes
        .iter_mut()
        .find(|f| f.nome == "piloto")
        .unwrap()
        .prazo_ms = Some(120);
    let mut b = banca_com(p);
    let (antes, depois) = medido(|| b.i.pedir_twist(b.piloto, GIRO));
    assert_eq!(b.em(antes + curto).aplicado, GIRO);
    let e = b.em(depois + curto + MS);
    assert_eq!(e.aplicado, [0.0; 3], "o prazo de 120 ms do piloto passou");
    assert_eq!(e.limitado_por, vec!["deadman"]);
    // O agente não tem prazo próprio: vale o de `[seguranca]`, 500 ms.
    let (antes, depois) = medido(|| b.i.pedir_twist(b.agente, FRENTE));
    assert_eq!(b.em(antes + b.prazo).aplicado, FRENTE);
    assert_eq!(b.em(depois + b.prazo + MS).aplicado, [0.0; 3]);
}

/// A parada nunca é barrada: vem da fonte de menor prioridade, com o piloto e o agente pedindo, e zera as rodas e a
/// velocidade da torre no ciclo seguinte. Quem tem prioridade volta a mandar no pedido seguinte.
#[test]
fn a_parada_de_qualquer_fonte_zera_tudo() {
    let mut b = banca();
    assert!(b.i.pedir_twist(b.piloto, GIRO));
    assert!(b.i.pedir_twist(b.agente, FRENTE));
    assert!(b.i.pedir_taxa_torre(b.piloto, 30.0, 0.0));
    let e = b.ja();
    assert_eq!(e.aplicado, GIRO);
    assert_eq!(b.de(id::TORRE_TAXA).len(), 1, "a torre gira");
    b.corpo.limpar();
    b.i.parar(FonteId::SEM_NOME);
    let e = b.ja();
    assert_eq!(e.aplicado, [0.0; 3], "o aplicado não vai a zero");
    assert!(
        e.limitado_por.is_empty(),
        "parar não é prazo vencido: {:?}",
        e.limitado_por
    );
    assert_eq!(
        nome(&e.dono_rodas),
        Some(FONTE_SEM_NOME),
        "o zero de quem parou"
    );
    let taxas = b.de(id::TORRE_TAXA);
    assert_eq!(taxas.len(), 1, "o quadro que para a torre, uma vez");
    // O quadro leva o yaw, um campo vazio e o pitch, dois bytes cada.
    assert_eq!(
        &taxas[0].dados[..6],
        [0; 6],
        "velocidade zero nos dois eixos"
    );
    // O pedido do agente de antes da parada não volta sozinho.
    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(b.ja().aplicado, [0.0; 3]);
    // E quem tem prioridade volta a mandar no pedido seguinte.
    assert!(b.i.pedir_twist(b.piloto, GIRO));
    assert_eq!(b.ja().aplicado, GIRO);
}

/// A parada de uma fonte que não tem as rodas na tabela também para: ela não é um pedido de rodas.
#[test]
fn a_parada_de_quem_nao_tem_as_rodas_tambem_para() {
    let mut p = Params::default();
    p.fontes.push(Fonte {
        nome: "voz".to_owned(),
        prioridade: 5,
        canais: vec![Canal::Som],
        prazo_ms: None,
        so_autonomo: false,
    });
    let mut b = banca_com(p);
    let voz = b.i.fonte(Some("voz"));
    assert!(!b.i.pode(voz, Canal::Rodas));
    assert!(
        !b.i.pedir_twist(voz, FRENTE),
        "sem o canal, o pedido não é guardado"
    );
    assert!(!b.i.pedir_torre(voz, 0.2, 0.0));
    assert!(!b.i.pedir_taxa_torre(voz, 30.0, 0.0));
    assert_eq!(b.ja().aplicado, [0.0; 3]);
    assert!(b.i.pedir_twist(b.piloto, GIRO));
    assert_eq!(b.ja().aplicado, GIRO);
    b.i.parar(voz);
    let e = b.ja();
    assert_eq!(e.aplicado, [0.0; 3]);
    assert_eq!(nome(&e.dono_rodas), None);
}

/// Aceite 8, no controle: o cérebro só conta com a autonomia ligada, e deixa de contar na hora em que ela desliga.
#[test]
fn o_cerebro_so_conta_com_a_autonomia_ligada() {
    let mut b = banca();
    assert!(!b.i.autonomia(), "sobe desligada");
    assert!(b.i.espera_a_autonomia(b.cerebro));
    assert!(!b.i.espera_a_autonomia(b.agente));
    assert!(
        !b.i.pedir_twist(b.cerebro, FRENTE),
        "o pedido vale com a autonomia desligada"
    );
    assert!(!b.i.pedir_taxa_torre(b.cerebro, 20.0, 0.0));
    assert!(!b.i.pedir_torre(b.cerebro, 0.2, 0.0));
    let e = b.ja();
    assert_eq!(e.aplicado, [0.0; 3]);
    assert!(!e.autonomia);
    b.i.definir_autonomia(true);
    assert!(b.i.pedir_twist(b.cerebro, FRENTE));
    let e = b.ja();
    assert_eq!(e.aplicado, FRENTE);
    assert!(e.autonomia);
    assert_eq!(nome(&e.dono_rodas), Some("cerebro"));
    // O agente está acima do cérebro.
    assert!(b.i.pedir_twist(b.agente, GIRO));
    assert_eq!(b.ja().aplicado, GIRO);
    b.i.parar(b.agente);
    assert!(b.i.pedir_twist(b.cerebro, FRENTE));
    // O zero do agente segura o cérebro enquanto vive.
    assert_eq!(b.ja().aplicado, [0.0; 3]);
}

/// Desligada a autonomia, o que o cérebro pediu deixa de valer na hora, e não volta ao religar.
#[test]
fn desligar_a_autonomia_apaga_o_que_o_cerebro_pediu() {
    let mut b = banca();
    b.i.definir_autonomia(true);
    assert!(b.i.pedir_twist(b.cerebro, FRENTE));
    assert!(b.i.pedir_taxa_torre(b.cerebro, 20.0, 0.0));
    assert_eq!(b.ja().aplicado, FRENTE);
    b.corpo.limpar();
    b.i.definir_autonomia(false);
    let e = b.ja();
    assert_eq!(
        e.aplicado, [0.0; 3],
        "o cérebro seguiu andando com a autonomia desligada"
    );
    assert_eq!((nome(&e.dono_rodas), nome(&e.dono_torre)), (None, None));
    b.i.definir_autonomia(true);
    let e = b.ja();
    assert_eq!(e.aplicado, [0.0; 3], "o pedido antigo do cérebro voltou");
    assert_eq!(nome(&e.dono_rodas), None);
}

/// Armar apaga os pedidos de todas as fontes: ligar não move o robô.
#[test]
fn armar_apaga_os_pedidos_de_todas_as_fontes() {
    let mut b = banca();
    b.i.pedir_armar(false);
    b.ja();
    assert!(b.i.pedir_twist(b.piloto, GIRO));
    assert!(b.i.pedir_twist(b.agente, FRENTE));
    assert!(b.i.pedir_taxa_torre(b.agente, 30.0, 0.0));
    assert!(b.i.pedir_torre(b.piloto, 0.2, 0.0));
    b.corpo.limpar();
    b.i.pedir_armar(true);
    let e = b.ja();
    assert_eq!(e.aplicado, [0.0; 3]);
    assert_eq!((nome(&e.dono_rodas), nome(&e.dono_torre)), (None, None));
    assert!(b.de(id::TORRE_ANGULOS).is_empty() && b.de(id::TORRE_TAXA).is_empty());
}

/// Aceite 10, no controle: na velocidade da torre também vale a prioridade, e o prazo a devolve.
#[test]
fn na_torre_a_velocidade_do_piloto_vence_e_o_prazo_a_devolve() {
    let mut b = banca();
    let taxa = |b: &Banca| {
        let q = b.de(id::TORRE_TAXA).pop().expect("há velocidade da torre");
        i16::from_le_bytes([q.dados[0], q.dados[1]])
    };
    assert!(b.i.pedir_taxa_torre(b.agente, 20.0, 0.0));
    let e = b.ja();
    assert_eq!(nome(&e.dono_torre), Some("agente"));
    assert_eq!(taxa(&b), 200, "décimos de grau por segundo");
    let (antes, depois) = medido(|| b.i.pedir_taxa_torre(b.piloto, -30.0, 0.0));
    std::thread::sleep(Duration::from_millis(150));
    assert!(b.i.pedir_taxa_torre(b.agente, 20.0, 0.0));
    let e = b.em(antes + b.prazo);
    assert_eq!(nome(&e.dono_torre), Some("piloto"));
    assert_eq!(taxa(&b), -300, "a torre segue a do agente");
    let e = b.em(depois + b.prazo + MS);
    assert_eq!(nome(&e.dono_torre), Some("agente"));
    assert_eq!(taxa(&b), 200, "a torre não voltou ao agente");
}

/// O alvo de ângulo é um só, e uma fonte menor não o troca enquanto a maior tem a torre; vencida a posse, qualquer uma
/// pode pedir, e a torre fica onde estava até lá.
#[test]
fn o_alvo_da_torre_so_e_trocado_por_quem_pode() {
    let mut p = Params::default();
    p.fontes
        .iter_mut()
        .find(|f| f.nome == "piloto")
        .unwrap()
        .prazo_ms = Some(80);
    let mut b = banca_com(p);
    assert!(b.i.pedir_torre(b.piloto, 0.2, 0.0));
    let e = b.ja();
    assert_eq!(nome(&e.dono_torre), Some("piloto"));
    assert_eq!(b.de(id::TORRE_ANGULOS).len(), 1);
    assert!(
        !b.i.pedir_torre(b.agente, -0.3, 0.0),
        "o piloto tem a torre"
    );
    assert_eq!(
        b.i.quem_supera(b.agente, Canal::Torre, Instant::now()),
        Some(b.piloto)
    );
    let antes = b.ja().torre;
    std::thread::sleep(Duration::from_millis(100));
    // A posse venceu: o alvo segue valendo, e ninguém tem a torre.
    let e = b.ja();
    assert_eq!(e.torre, antes, "a torre fica onde está");
    assert_eq!(nome(&e.dono_torre), None);
    assert!(
        b.i.pedir_torre(b.agente, -0.3, 0.0),
        "vencida a posse, o agente pode"
    );
    let e = b.ja();
    assert_eq!(nome(&e.dono_torre), Some("agente"));
    assert_ne!(e.torre, antes);
    // O piloto retoma na hora: é maior.
    assert!(b.i.pedir_torre(b.piloto, 0.1, 0.0));
    assert_eq!(nome(&b.ja().dono_torre), Some("piloto"));
}

fn ciclos_parado(b: &mut Banca, de: Instant) {
    // Bem mais que o reenvio do alvo (100 ms): se houvesse alvo, ele sairia.
    for n in 0..30u64 {
        b.em(de + Duration::from_millis(20 * n));
    }
}

/// Aceite 11: uma fonte pôs um alvo de ângulo, outra moveu a torre por velocidade e parou. A torre fica onde parou:
/// nenhum alvo antigo é reenviado.
#[test]
fn o_alvo_de_uma_fonte_nao_volta_depois_da_velocidade_de_outra() {
    let mut b = banca();
    assert!(b.i.pedir_torre(b.agente, 0.3, 0.0));
    b.ja();
    assert_eq!(b.de(id::TORRE_ANGULOS).len(), 1, "o alvo do agente saiu");
    assert!(b.i.pedir_taxa_torre(b.piloto, 30.0, 0.0));
    b.ja();
    assert!(b.i.pedir_taxa_torre(b.piloto, 0.0, 0.0));
    b.corpo.limpar();
    ciclos_parado(&mut b, Instant::now());
    assert!(
        b.de(id::TORRE_ANGULOS).is_empty(),
        "sai um quadro de ângulo depois de a velocidade ir a zero: {:?}",
        b.de(id::TORRE_ANGULOS)
    );
}

/// O mesmo, pelo caminho da herança: a fonte menor pediu velocidade com a maior ainda dona da torre pelo alvo. O pedido
/// fica guardado sem mexer no alvo de quem manda; quando a posse vence e a velocidade passa a valer, o alvo antigo some.
#[test]
fn a_velocidade_que_herda_a_torre_apaga_o_alvo_antigo() {
    let mut b = banca();
    let (_, depois) = medido(|| b.i.pedir_torre(b.piloto, 0.3, 0.0));
    b.ja();
    assert_eq!(b.de(id::TORRE_ANGULOS).len(), 1);
    std::thread::sleep(Duration::from_millis(150));
    let (_, depois_taxa) = medido(|| b.i.pedir_taxa_torre(b.agente, 20.0, 0.0));
    b.corpo.limpar();
    let e = b.ja();
    assert_eq!(
        nome(&e.dono_torre),
        Some("piloto"),
        "ainda é do piloto, pelo alvo"
    );
    assert!(
        b.de(id::TORRE_TAXA).is_empty(),
        "a velocidade do agente não vale ainda"
    );
    assert!(e.torre.is_some(), "e o alvo do piloto segue");
    // A posse do piloto venceu, e a velocidade do agente ainda vive: ele herda a torre.
    let e = b.em(depois + b.prazo + MS);
    assert_eq!(nome(&e.dono_torre), Some("agente"));
    assert_eq!(b.de(id::TORRE_TAXA).len(), 1);
    // A velocidade do agente vence também. A torre para onde está.
    b.corpo.limpar();
    let vencida = depois_taxa + b.prazo + MS;
    ciclos_parado(&mut b, vencida);
    assert!(
        b.de(id::TORRE_ANGULOS).is_empty(),
        "o alvo antigo do piloto voltou: {:?}",
        b.de(id::TORRE_ANGULOS)
    );
}
