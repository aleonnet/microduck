//! O agente contra um corpo de mentira: cada habilidade cumpre, vence, é interrompida ou é recusada como a tabela manda, e a
//! parada nunca é barrada. O núcleo do agente não tem rede: o que entra é o pedido validado, o estado do corpo e as
//! detecções, e o que sai é o que ele pede ao `rms1d` e como cada habilidade terminou.

use std::time::{Duration, Instant};

use rms1_agente::agente::{Acao, Agente, Caixa, Como, Corpo, Pedido};
use rms1_agente::tabela::Tabela;
use serde_json::{Value, json};

fn agente() -> Agente {
    Agente::novo(Tabela::padrao().expect("a tabela que vem com o agente"))
}

fn pedido(ferramenta: &str, parametros: Value) -> Pedido {
    Pedido {
        numero: 7,
        ferramenta: ferramenta.to_owned(),
        parametros: parametros.as_object().cloned().unwrap_or_default(),
    }
}

fn armado_em(x: f64, y: f64) -> Corpo {
    Corpo {
        armado: true,
        posicao: [x, y],
    }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// O que foi pedido ao corpo: o método e os parâmetros de cada linha.
fn ao_corpo(acoes: &[Acao]) -> Vec<(String, Value)> {
    acoes
        .iter()
        .filter_map(|a| match a {
            Acao::Corpo(v) => Some((
                v["method"].as_str().unwrap().to_owned(),
                v.get("params").cloned().unwrap_or(Value::Null),
            )),
            Acao::Terminou(_) => None,
        })
        .collect()
}

/// Como cada habilidade terminou: a ferramenta e o desfecho.
fn desfechos(acoes: &[Acao]) -> Vec<(String, Como)> {
    acoes
        .iter()
        .filter_map(|a| match a {
            Acao::Terminou(d) => Some((d.ferramenta.clone(), d.como)),
            Acao::Corpo(_) => None,
        })
        .collect()
}

/// O que o agente manda para soltar as rodas: o pedido de andar, a zero. Não é `robot.stop`, que pararia quem pilota.
fn rodas_soltas() -> (String, Value) {
    (
        "robot.move".to_owned(),
        json!({"vx": 0.0, "vy": 0.0, "vyaw": 0.0}),
    )
}

fn pessoa_em(x: f64, y: f64) -> Vec<Caixa> {
    vec![Caixa {
        rotulo: "person".to_owned(),
        centro: [x, y],
        nota: 0.9,
    }]
}

#[test]
fn a_tabela_que_vem_com_o_agente_vale_e_a_errada_e_recusada_pelo_nome() {
    let t = Tabela::padrao().unwrap();
    let nomes: Vec<&str> = t.habilidades.iter().map(|h| h.ferramenta()).collect();
    assert_eq!(nomes, ["stop", "move", "look_at", "search"]);
    // O pedido contínuo tem de ser renovado bem dentro do prazo do `rms1d` (500 ms), e a velocidade tem de caber no corpo.
    assert!(t.conferir_com_o_corpo(500, 3.5).is_ok());
    let curto = t.conferir_com_o_corpo(150, 3.5).unwrap_err();
    assert!(curto.contains("renovar_ms"), "{curto}");
    let lento = t.conferir_com_o_corpo(500, 0.1).unwrap_err();
    assert!(lento.contains("velocidade"), "{lento}");
    for (errada, nome) in [
        (
            "[geral]\nrenovar_ms = 100\ncorpo_vale_ms = 1000\n[[habilidade]]\nfaz = \"parar\"\nferramenta = \"stop\"\n[[habilidade]]\nfaz = \"parar\"\nferramenta = \"stop\"\n",
            "stop",
        ),
        (
            "[geral]\nrenovar_ms = 100\ncorpo_vale_ms = 1000\n[[habilidade]]\nfaz = \"voar\"\nferramenta = \"fly\"\n",
            "voar",
        ),
        (
            "[geral]\nrenovar_ms = 0\ncorpo_vale_ms = 1000\n[[habilidade]]\nfaz = \"parar\"\nferramenta = \"stop\"\n",
            "renovar_ms",
        ),
        ("[geral]\nrenovar_ms = 100\ncorpo_vale_ms = 1000\n", "parar"),
    ] {
        let e = Tabela::ler(errada).unwrap_err();
        assert!(e.contains(nome), "{nome}: {e}");
    }
}

#[test]
fn mover_anda_a_distancia_pela_odometria_renovando_o_pedido_e_para() {
    let mut a = agente();
    let t0 = Instant::now();
    a.corpo(armado_em(1.0, 2.0), t0);
    let acoes = a.pedido(
        pedido("move", json!({"direction": "forward", "meters": 0.5})),
        t0,
    );
    // O primeiro pedido sai na hora, na velocidade da tabela, para a frente.
    assert_eq!(
        ao_corpo(&acoes),
        [(
            "robot.move".to_owned(),
            json!({"vx": 0.3, "vy": 0.0, "vyaw": 0.0})
        )]
    );
    assert!(desfechos(&acoes).is_empty());
    // Antes do prazo de renovar, nada; passado ele, o mesmo pedido de novo.
    assert!(a.tique(t0 + ms(50)).is_empty());
    assert_eq!(ao_corpo(&a.tique(t0 + ms(100))).len(), 1);
    // O corpo andou 0,3 m: segue.
    a.corpo(armado_em(1.3, 2.0), t0 + ms(1000));
    let segue = a.tique(t0 + ms(1000));
    assert_eq!(ao_corpo(&segue)[0].0, "robot.move");
    assert!(desfechos(&segue).is_empty());
    // Passou de 1,67 s, o tempo da distância na velocidade pedida: com a odometria andando, quem decide é a distância.
    assert!(desfechos(&a.tique(t0 + ms(1800))).is_empty());
    // Chegou a 0,5 m: solta as rodas, e termina cumprida.
    a.corpo(armado_em(1.5, 2.0), t0 + ms(2000));
    let fim = a.tique(t0 + ms(2000));
    assert_eq!(ao_corpo(&fim), [rodas_soltas()]);
    assert_eq!(desfechos(&fim), [("move".to_owned(), Como::Cumprida)]);
    // Terminada, não pede mais nada.
    assert!(a.tique(t0 + ms(2200)).is_empty());
    // Para a esquerda, o `y` do corpo é positivo.
    a.corpo(armado_em(1.5, 2.0), t0 + ms(3000));
    let esquerda = a.pedido(
        pedido("move", json!({"direction": "left", "meters": 0.2})),
        t0 + ms(3000),
    );
    assert_eq!(
        ao_corpo(&esquerda)[0].1,
        json!({"vx": 0.0, "vy": 0.3, "vyaw": 0.0})
    );
}

#[test]
fn mover_sem_odometria_anda_pelo_tempo_e_o_que_nao_chega_vence() {
    // O corpo não diz onde está (a posição não muda): anda pelo tempo da distância na velocidade da tabela.
    let mut a = agente();
    let t0 = Instant::now();
    a.corpo(armado_em(0.0, 0.0), t0);
    a.pedido(
        pedido("move", json!({"direction": "back", "meters": 0.6})),
        t0,
    );
    a.corpo(armado_em(0.0, 0.0), t0 + ms(1900));
    assert!(desfechos(&a.tique(t0 + ms(1900))).is_empty());
    a.corpo(armado_em(0.0, 0.0), t0 + ms(2000));
    let fim = a.tique(t0 + ms(2000));
    assert_eq!(ao_corpo(&fim), [rodas_soltas()]);
    assert_eq!(desfechos(&fim), [("move".to_owned(), Como::Cumprida)]);

    // A odometria andou um pouco e parou (o robô encostou em algo): no teto, solta as rodas e diz que venceu, com o quanto andou.
    let mut a = agente();
    a.corpo(armado_em(0.0, 0.0), t0);
    a.pedido(
        pedido("move", json!({"direction": "forward", "meters": 0.6})),
        t0,
    );
    a.corpo(armado_em(0.1, 0.0), t0 + ms(500));
    for n in (600..5000).step_by(100) {
        a.corpo(armado_em(0.1, 0.0), t0 + ms(n));
        assert!(
            desfechos(&a.tique(t0 + ms(n))).is_empty(),
            "terminou aos {n} ms, antes do teto"
        );
    }
    a.corpo(armado_em(0.1, 0.0), t0 + ms(5000));
    let fim = a.tique(t0 + ms(5000));
    assert_eq!(ao_corpo(&fim), [rodas_soltas()]);
    let Acao::Terminou(d) = fim.last().unwrap() else {
        panic!("sem desfecho")
    };
    assert_eq!((d.ferramenta.as_str(), d.como), ("move", Como::Vencida));
    assert!(d.motivo.contains("0.10"), "{}", d.motivo);
}

#[test]
fn parar_nunca_e_barrado_e_encerra_o_que_esta_em_curso() {
    // Sem corpo respondendo: a parada sai assim mesmo.
    let mut a = agente();
    let t0 = Instant::now();
    let acoes = a.pedido(pedido("stop", json!({})), t0);
    assert_eq!(ao_corpo(&acoes), [("robot.stop".to_owned(), Value::Null)]);
    assert_eq!(desfechos(&acoes), [("stop".to_owned(), Como::Cumprida)]);
    // A parada é uma requisição, com número: o `rms1d` não aplica `robot.stop` mandado como notificação.
    let Acao::Corpo(linha) = &acoes[0] else {
        panic!("sem pedido ao corpo")
    };
    assert!(linha["id"].is_u64(), "{linha}");

    // Com o corpo andando: para, o que estava em curso termina interrompido, e a parada termina cumprida.
    a.corpo(armado_em(0.0, 0.0), t0);
    a.pedido(
        pedido("move", json!({"direction": "forward", "meters": 2.0})),
        t0,
    );
    let parada = Pedido {
        numero: 8,
        ..pedido("stop", json!({}))
    };
    let acoes = a.pedido(parada, t0 + ms(300));
    assert_eq!(ao_corpo(&acoes), [("robot.stop".to_owned(), Value::Null)]);
    // Cada desfecho leva o número do pedido dele na estação: o do que foi interrompido, e o da parada.
    let numeros: Vec<u64> = acoes
        .iter()
        .filter_map(|a| match a {
            Acao::Terminou(d) => Some(d.numero),
            Acao::Corpo(_) => None,
        })
        .collect();
    assert_eq!(numeros, [7, 8]);
    assert_eq!(
        desfechos(&acoes),
        [
            ("move".to_owned(), Como::Interrompida),
            ("stop".to_owned(), Como::Cumprida)
        ]
    );
    assert!(
        a.tique(t0 + ms(500)).is_empty(),
        "o corpo seguiu recebendo pedido depois da parada"
    );
}

#[test]
fn o_que_nao_pode_ser_cumprido_e_recusado_sem_mexer_no_corpo() {
    let t0 = Instant::now();
    let mover = || pedido("move", json!({"direction": "forward", "meters": 0.5}));
    // Cada caso: o que foi feito antes do pedido, o pedido, e a palavra que o motivo traz.
    type Antes = fn(&mut Agente, Instant);
    let casos: [(Antes, Pedido, &str); 6] = [
        (|_, _| {}, mover(), "não responde"),
        (
            |a, t| {
                a.corpo(
                    Corpo {
                        armado: false,
                        posicao: [0.0; 2],
                    },
                    t,
                )
            },
            mover(),
            "armado",
        ),
        (
            // O estado do corpo é velho: o `rms1d` saiu.
            |a, t| a.corpo(armado_em(0.0, 0.0), t - ms(1500)),
            mover(),
            "não responde",
        ),
        (
            |a, t| a.corpo(armado_em(0.0, 0.0), t),
            pedido("dance", json!({})),
            "dance",
        ),
        (
            |a, t| a.corpo(armado_em(0.0, 0.0), t),
            pedido("move", json!({"direction": "up", "meters": 0.5})),
            "direction",
        ),
        (
            |a, t| a.corpo(armado_em(0.0, 0.0), t),
            pedido("move", json!({"direction": "forward"})),
            "meters",
        ),
    ];
    for (antes, p, palavra) in casos {
        let mut a = agente();
        antes(&mut a, t0);
        let acoes = a.pedido(p, t0);
        assert!(ao_corpo(&acoes).is_empty(), "{palavra}: mexeu no corpo");
        let [Acao::Terminou(d)] = acoes.as_slice() else {
            panic!("{palavra}: {} ações", acoes.len())
        };
        assert_eq!(d.como, Como::Recusada, "{palavra}");
        assert!(d.motivo.contains(palavra), "{palavra}: {}", d.motivo);
        assert!(
            a.tique(t0 + ms(200)).is_empty(),
            "{palavra}: ficou em curso"
        );
    }
}

#[test]
fn um_pedido_novo_encerra_o_anterior_e_o_corpo_desarmado_ou_mudo_interrompe() {
    let mut a = agente();
    let t0 = Instant::now();
    a.corpo(armado_em(0.0, 0.0), t0);
    a.pedido(
        pedido("move", json!({"direction": "forward", "meters": 2.0})),
        t0,
    );
    // Outro pedido: as rodas são soltas, o anterior termina interrompido, e o novo começa.
    let acoes = a.pedido(
        pedido("move", json!({"direction": "back", "meters": 0.3})),
        t0 + ms(400),
    );
    assert_eq!(
        ao_corpo(&acoes),
        [
            rodas_soltas(),
            (
                "robot.move".to_owned(),
                json!({"vx": -0.3, "vy": 0.0, "vyaw": 0.0})
            )
        ]
    );
    assert_eq!(desfechos(&acoes), [("move".to_owned(), Como::Interrompida)]);
    // Quem pilota desarmou o robô no meio: o agente larga a habilidade, e não pede mais nada.
    a.corpo(
        Corpo {
            armado: false,
            posicao: [0.0; 2],
        },
        t0 + ms(600),
    );
    let acoes = a.tique(t0 + ms(600));
    assert_eq!(ao_corpo(&acoes), [rodas_soltas()]);
    assert_eq!(desfechos(&acoes), [("move".to_owned(), Como::Interrompida)]);
    assert!(a.tique(t0 + ms(800)).is_empty());

    // O corpo para de dizer o estado (o `rms1d` caiu): idem.
    let mut a = agente();
    a.corpo(armado_em(0.0, 0.0), t0);
    a.pedido(
        pedido("move", json!({"direction": "forward", "meters": 2.0})),
        t0,
    );
    assert!(desfechos(&a.tique(t0 + ms(900))).is_empty());
    let acoes = a.tique(t0 + ms(1100));
    assert_eq!(ao_corpo(&acoes), [rodas_soltas()]);
    assert_eq!(desfechos(&acoes), [("move".to_owned(), Como::Interrompida)]);
}

/// `robot.stop` é só da ferramenta de parar: no `rms1d` ela apaga os pedidos de todas as fontes, e mandada no fim de uma
/// habilidade pararia quem pilota. Cada fim de habilidade, por qualquer caminho, solta só o canal dela.
#[test]
fn nenhum_fim_de_habilidade_manda_robot_stop() {
    let t0 = Instant::now();
    let mover = || pedido("move", json!({"direction": "forward", "meters": 0.5}));
    type Fim = fn(&mut Agente, Instant) -> Vec<Acao>;
    let fins: [(&str, Pedido, Fim); 9] = [
        ("cumprida", mover(), |a, t| {
            a.corpo(armado_em(0.5, 0.0), t + ms(2000));
            a.tique(t + ms(2000))
        }),
        ("vencida", mover(), |a, t| {
            a.corpo(armado_em(0.1, 0.0), t + ms(5000));
            a.tique(t + ms(5000))
        }),
        ("largada", mover(), |a, t| a.largar(7, t + ms(300))),
        ("trocada", mover(), |a, t| {
            a.pedido(pedido("look_at", json!({"target": "me"})), t + ms(300))
        }),
        ("desarmado", mover(), |a, t| {
            a.corpo(
                Corpo {
                    armado: false,
                    posicao: [0.0; 2],
                },
                t + ms(300),
            );
            a.tique(t + ms(300))
        }),
        ("mudo", mover(), |a, t| a.tique(t + ms(1100))),
        (
            "torre vencida",
            pedido("search", json!({"target": "dog"})),
            |a, t| {
                a.corpo(armado_em(0.0, 0.0), t + ms(20000));
                a.tique(t + ms(20000))
            },
        ),
        (
            "torre cumprida",
            pedido("search", json!({"target": "me"})),
            |a, t| a.deteccoes(&pessoa_em(0.5, 0.5), t + ms(300)),
        ),
        (
            "torre largada",
            pedido("search", json!({"target": "dog"})),
            |a, t| a.largar(7, t + ms(300)),
        ),
    ];
    for (caso, p, fim) in fins {
        let mut a = agente();
        a.corpo(armado_em(0.0, 0.0), t0);
        let andava = p.ferramenta == "move";
        a.pedido(p, t0);
        let acoes = fim(&mut a, t0);
        assert_eq!(desfechos(&acoes).len(), 1, "{caso}: não terminou");
        let foi = ao_corpo(&acoes);
        assert!(
            foi.iter().all(|(m, _)| m != "robot.stop"),
            "{caso}: a lista do que foi ao corpo traz robot.stop: {foi:?}"
        );
        // O canal da habilidade é solto primeiro: as rodas a zero, ou a torre parada.
        let solto = if andava {
            rodas_soltas()
        } else {
            ("s1.gimbal".to_owned(), json!({"action": "stop"}))
        };
        assert_eq!(foi.first(), Some(&solto), "{caso}");
    }
}

#[test]
fn olhar_gira_a_torre_ate_o_alvo_ficar_no_meio_do_quadro() {
    let mut a = agente();
    let t0 = Instant::now();
    a.corpo(armado_em(0.0, 0.0), t0);
    // "me" é uma pessoa, pela tabela de alvos.
    let acoes = a.pedido(pedido("look_at", json!({"target": "me"})), t0);
    assert!(
        ao_corpo(&acoes).is_empty(),
        "antes de ver o alvo, a torre fica"
    );
    // O alvo à direita e acima do meio: a torre vai para a direita e para cima, no sinal da DJI (yaw à direita, pitch
    // para cima), tanto mais depressa quanto mais longe do meio.
    let acoes = a.deteccoes(&pessoa_em(0.75, 0.25), t0 + ms(100));
    let (metodo, p) = &ao_corpo(&acoes)[0];
    assert_eq!(
        (metodo.as_str(), &p["action"]),
        ("s1.gimbal", &json!("speed"))
    );
    let (longe_yaw, longe_pitch) = (p["yaw"].as_f64().unwrap(), p["pitch"].as_f64().unwrap());
    assert!(longe_yaw > 0.0 && longe_pitch > 0.0, "{p}");
    let acoes = a.deteccoes(&pessoa_em(0.6, 0.5), t0 + ms(200));
    let perto = ao_corpo(&acoes)[0].1["yaw"].as_f64().unwrap();
    assert!(perto > 0.0 && perto < longe_yaw, "{perto} e {longe_yaw}");
    // Outra coisa no quadro não é o alvo: a torre para, e a contagem recomeça.
    let cachorro = vec![Caixa {
        rotulo: "dog".to_owned(),
        centro: [0.5, 0.5],
        nota: 0.9,
    }];
    let acoes = a.deteccoes(&cachorro, t0 + ms(300));
    assert_eq!(ao_corpo(&acoes)[0].1, json!({"action": "stop"}));
    // No meio por três detecções seguidas: a torre para, e termina cumprida.
    assert!(desfechos(&a.deteccoes(&pessoa_em(0.52, 0.49), t0 + ms(400))).is_empty());
    assert!(desfechos(&a.deteccoes(&pessoa_em(0.51, 0.5), t0 + ms(500))).is_empty());
    let fim = a.deteccoes(&pessoa_em(0.5, 0.5), t0 + ms(600));
    assert_eq!(desfechos(&fim), [("look_at".to_owned(), Como::Cumprida)]);
    assert_eq!(ao_corpo(&fim)[0].1, json!({"action": "stop"}));

    // O alvo nunca aparece: no teto da tabela, vence, dizendo o que não viu.
    let mut a = agente();
    a.corpo(armado_em(0.0, 0.0), t0);
    a.pedido(pedido("look_at", json!({"target": "the red ball"})), t0);
    for n in (100..8000).step_by(100) {
        a.corpo(armado_em(0.0, 0.0), t0 + ms(n));
        assert!(desfechos(&a.tique(t0 + ms(n))).is_empty(), "aos {n} ms");
    }
    a.corpo(armado_em(0.0, 0.0), t0 + ms(8000));
    let fim = a.tique(t0 + ms(8000));
    let Acao::Terminou(d) = fim.last().unwrap() else {
        panic!("sem desfecho")
    };
    assert_eq!(d.como, Como::Vencida);
    assert!(d.motivo.contains("the red ball"), "{}", d.motivo);
    // "ball" casa com o rótulo "sports ball" do detector.
    let mut a = agente();
    a.corpo(armado_em(0.0, 0.0), t0);
    a.pedido(pedido("look_at", json!({"target": "the red ball"})), t0);
    let bola = vec![Caixa {
        rotulo: "sports ball".to_owned(),
        centro: [0.2, 0.5],
        nota: 0.6,
    }];
    let acoes = a.deteccoes(&bola, t0 + ms(100));
    assert!(ao_corpo(&acoes)[0].1["yaw"].as_f64().unwrap() < 0.0);
}

#[test]
fn procurar_varre_com_a_torre_e_para_quando_acha() {
    let mut a = agente();
    let t0 = Instant::now();
    a.corpo(armado_em(0.0, 0.0), t0);
    let acoes = a.pedido(pedido("search", json!({"target": "dog"})), t0);
    // Começa a varrer na hora, para um lado, na velocidade da tabela.
    let (metodo, p) = &ao_corpo(&acoes)[0];
    assert_eq!(metodo, "s1.gimbal");
    let ida = p["yaw"].as_f64().unwrap();
    assert_eq!((ida.abs(), p["pitch"].as_f64()), (40.0, Some(0.0)));
    // Meia varredura depois, volta pelo outro lado; e segue renovando o pedido.
    let mut lados = vec![ida.signum()];
    for n in (100..9000).step_by(100) {
        a.corpo(armado_em(0.0, 0.0), t0 + ms(n));
        for (_, p) in ao_corpo(&a.tique(t0 + ms(n))) {
            let lado = p["yaw"].as_f64().unwrap().signum();
            if lados.last() != Some(&lado) {
                lados.push(lado);
            }
        }
    }
    assert_eq!(lados, [ida.signum(), -ida.signum(), ida.signum()], "em 9 s");
    // Uma pessoa não é o cachorro; o cachorro é: a torre para, e termina cumprida.
    assert!(desfechos(&a.deteccoes(&pessoa_em(0.5, 0.5), t0 + ms(9050))).is_empty());
    let cachorro = vec![Caixa {
        rotulo: "dog".to_owned(),
        centro: [0.8, 0.5],
        nota: 0.7,
    }];
    let fim = a.deteccoes(&cachorro, t0 + ms(9100));
    assert_eq!(ao_corpo(&fim)[0].1, json!({"action": "stop"}));
    assert_eq!(desfechos(&fim), [("search".to_owned(), Como::Cumprida)]);

    // Não achou até o teto: para a torre e vence.
    let mut a = agente();
    a.corpo(armado_em(0.0, 0.0), t0);
    a.pedido(pedido("search", json!({"target": "dog"})), t0);
    let mut fim = Vec::new();
    for n in (100..=20000).step_by(100) {
        a.corpo(armado_em(0.0, 0.0), t0 + ms(n));
        fim = a.tique(t0 + ms(n));
        if !desfechos(&fim).is_empty() {
            assert_eq!(n, 20000, "venceu antes do teto");
        }
    }
    assert_eq!(desfechos(&fim), [("search".to_owned(), Como::Vencida)]);
    assert_eq!(ao_corpo(&fim)[0].1, json!({"action": "stop"}));
}

#[test]
fn a_estacao_manda_largar_o_pedido_em_curso() {
    let mut a = agente();
    let t0 = Instant::now();
    a.corpo(armado_em(0.0, 0.0), t0);
    a.pedido(
        pedido("move", json!({"direction": "forward", "meters": 2.0})),
        t0,
    );
    // A ordem de largar outro pedido não mexe no que está em curso.
    assert!(a.largar(9, t0 + ms(200)).is_empty());
    assert_eq!(a.em_curso(), Some("move"));
    // A do pedido em curso solta as rodas, e ele termina interrompido.
    let acoes = a.largar(7, t0 + ms(300));
    assert_eq!(ao_corpo(&acoes), [rodas_soltas()]);
    assert_eq!(desfechos(&acoes), [("move".to_owned(), Como::Interrompida)]);
    assert!(a.tique(t0 + ms(500)).is_empty());
    // Sem nada em curso, não há o que largar.
    assert!(a.largar(7, t0 + ms(600)).is_empty());
}
