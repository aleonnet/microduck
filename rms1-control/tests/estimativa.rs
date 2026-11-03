//! O cálculo próprio em modo sombra, com casos montados: um teste para cada parte da mudança.

use rms1_control::estimativa::{Calculo, Estimativa};
use rms1_control::telemetria::{BIT_PARADO, Medidas};
use rms1d_params::Params;

fn parado_em_pe() -> Medidas {
    Medidas {
        velocidade_solo: [0.0; 3],
        velocidade_corpo: [0.0; 3],
        rodas_rpm: [0.0; 4],
        rodas_angulo_grau: [0.0; 4],
        rodas_estado: [0; 4],
        posicao: [0.0; 3],
        aceleracao: [0.0, 0.0, 1.0],
        giro: [0.0; 3],
        rumo: 0.0,
        inclinacao: 0.0,
        rolagem: 0.0,
        volts: 12.0,
        percentual: 90.0,
        corrente_a: 0.0,
        temperatura_c: 30.0,
        relogio_ms: 1000,
    }
}

/// `n` pacotes, a cada 20 ms, como o robô manda, mudando o que `mudar` diz.
fn pacotes(
    e: &mut Estimativa,
    base: &Medidas,
    n: u32,
    estado: u32,
    mut mudar: impl FnMut(u32, &mut Medidas),
) -> Vec<Calculo> {
    (0..n)
        .map(|i| {
            let mut m = *base;
            m.relogio_ms = base.relogio_ms + 20 * i;
            mudar(i, &mut m);
            e.passo(&m, estado)
        })
        .collect()
}

fn nova() -> (Estimativa, Params) {
    let p = Params::default();
    (Estimativa::new(&p.estimativa, &p.geometria), p)
}

/// rpm de andar para a frente a `v` m/s, pela tabela física.
fn rpm_de(v: f64, p: &Params) -> f64 {
    v / p.geometria.raio_roda_m * 60.0 / std::f64::consts::TAU
}

/// Critério 16: inclinado e parado na mão, o controlador acusa 3 m/s; as rodas não andam, e a nossa velocidade não sobe.
#[test]
fn inclinado_parado_a_velocidade_do_controlador_nao_entra_na_nossa() {
    let (mut e, _) = nova();
    let base = Medidas {
        velocidade_corpo: [3.0, 0.5, 0.0],
        inclinacao: 30.0,
        aceleracao: [0.5, 0.0, 0.87],
        ..parado_em_pe()
    };
    let c = pacotes(&mut e, &base, 50, 0, |_, _| {});
    for x in &c {
        assert!(
            x.velocidade_corpo[0].hypot(x.velocidade_corpo[1]) < 0.1,
            "{x:?}"
        );
    }
    assert!(
        c.last().unwrap().levantado,
        "a discordância por 1 s marca levantado"
    );
}

/// Critério 17: parado pelo controlador, o ruído das rodas (até 32 rpm, medido) não vira velocidade.
#[test]
fn parado_pelo_controlador_a_velocidade_e_zero_apesar_do_ruido() {
    let (mut e, _) = nova();
    let c = pacotes(&mut e, &parado_em_pe(), 50, BIT_PARADO, |i, m| {
        let r = if i % 2 == 0 { 32.0 } else { -21.0 };
        m.rodas_rpm = [r, -r, r, -r];
    });
    for x in &c {
        assert_eq!(x.velocidade_corpo, [0.0, 0.0], "{x:?}");
        assert_eq!(x.giro_rodas, 0.0);
    }
}

/// Sem o bit de parado, o mesmo ruído aparece: é o que torna o teste de cima uma prova.
#[test]
fn sem_o_bit_de_parado_o_ruido_aparece() {
    let (mut e, _) = nova();
    let c = pacotes(&mut e, &parado_em_pe(), 5, 0, |_, m| {
        m.rodas_rpm = [32.0, 32.0, 32.0, 32.0];
    });
    assert!(c.last().unwrap().velocidade_corpo[0] > 0.1);
}

/// Andando reto a 0,4 m/s, com o rumo a 90°: a velocidade é a das rodas, e a posição anda em y no mundo.
#[test]
fn andando_a_velocidade_e_a_das_rodas_e_a_posicao_segue_o_rumo() {
    let (mut e, p) = nova();
    let r = rpm_de(0.4, &p);
    let base = Medidas {
        rodas_rpm: [r; 4],
        velocidade_corpo: [0.4, 0.0, 0.0],
        rumo: 90.0,
        ..parado_em_pe()
    };
    let c = pacotes(&mut e, &base, 51, 0, |_, _| {});
    let u = c.last().unwrap();
    assert!((u.velocidade_corpo[0] - 0.4).abs() < 1e-9, "{u:?}");
    assert!(u.velocidade_corpo[1].abs() < 1e-9);
    assert!((u.posicao[1] - 0.4).abs() < 1e-6, "1 s a 0,4 m/s: {u:?}");
    assert!(u.posicao[0].abs() < 1e-6);
    assert!(!u.levantado, "concordam: não está levantado");
}

/// A média é em tempo: depois de um buraco de 600 ms, a amostra parada de antes não puxa a velocidade para baixo.
#[test]
fn depois_de_um_buraco_a_media_nao_arrasta_amostra_velha() {
    let (mut e, p) = nova();
    let _ = pacotes(&mut e, &parado_em_pe(), 10, 0, |_, _| {});
    let r = rpm_de(0.3, &p);
    let andando = Medidas {
        rodas_rpm: [r; 4],
        relogio_ms: parado_em_pe().relogio_ms + 9 * 20 + 600,
        ..parado_em_pe()
    };
    let c = e.passo(&andando, 0);
    assert!((c.velocidade_corpo[0] - 0.3).abs() < 1e-9, "{c:?}");
}

/// Levantado por 300 ms marca; abaixo do limiar de soltar por 500 ms solta, e não antes.
#[test]
fn levantado_marca_e_solta_com_os_tempos_do_arquivo() {
    let (mut e, p) = nova();
    let l = p.estimativa;
    let no_ar = Medidas {
        velocidade_corpo: [1.0, 0.0, 0.0],
        ..parado_em_pe()
    };
    let antes = (l.levantado_tempo_ms / 20) as u32;
    let c = pacotes(&mut e, &no_ar, antes + 2, 0, |_, _| {});
    assert!(!c[antes as usize - 1].levantado, "antes do tempo");
    assert!(c.last().unwrap().levantado);
    let fim = no_ar.relogio_ms + 20 * (antes + 2);
    let soltar = (l.soltar_tempo_ms / 20) as u32;
    let c = pacotes(
        &mut e,
        &Medidas {
            relogio_ms: fim,
            ..parado_em_pe()
        },
        soltar + 2,
        0,
        |_, _| {},
    );
    assert!(
        c[soltar as usize - 1].levantado,
        "ainda dentro do tempo de soltar"
    );
    assert!(!c.last().unwrap().levantado);
}

/// Critério 20: a atitude presa em 24° com o robô parado e em pé avisa; os 2° a 3° medidos parado não avisam.
#[test]
fn atitude_presa_com_o_robo_quieto_avisa_e_o_desvio_normal_nao() {
    for (inclinacao, rolagem, avisa) in [(-23.9, -24.3, true), (1.9, 1.7, false)] {
        let (mut e, _) = nova();
        let m = Medidas {
            inclinacao,
            rolagem,
            ..parado_em_pe()
        };
        let c = e.passo(&m, BIT_PARADO);
        assert_eq!(c.divergente, avisa, "{inclinacao} {rolagem}: {c:?}");
        let d = c.divergencia.unwrap();
        if avisa {
            assert!(d > 30.0, "{d}");
        }
    }
}

/// Freando, a aceleração inclina sem mudar de módulo: sem o bit de parado não é quieto, e não há conferência.
#[test]
fn freando_nao_e_quieto() {
    let (mut e, _) = nova();
    let m = Medidas {
        aceleracao: [0.2, 0.0, 0.98],
        ..parado_em_pe()
    };
    let c = e.passo(&m, 0);
    assert_eq!(c.divergencia, None);
    assert!(!c.divergente);
}

/// Em pé, a gravidade pela atitude é a do contrato.
#[test]
fn em_pe_a_gravidade_e_menos_um_em_z() {
    let (mut e, _) = nova();
    let g = e.passo(&parado_em_pe(), BIT_PARADO).gravidade;
    for (a, b) in g.iter().zip([0.0, 0.0, -1.0]) {
        assert!((a - b).abs() < 1e-12, "{g:?}");
    }
}

/// Um relógio que recomeça não vira deslocamento: acima de `lacuna_ms`, não integra.
#[test]
fn relogio_que_recomeca_nao_vira_deslocamento() {
    let (mut e, p) = nova();
    let r = rpm_de(0.5, &p);
    let andando = Medidas {
        rodas_rpm: [r; 4],
        velocidade_corpo: [0.5, 0.0, 0.0],
        ..parado_em_pe()
    };
    let antes = e.passo(&andando, 0).posicao;
    let depois = e
        .passo(
            &Medidas {
                relogio_ms: andando.relogio_ms.wrapping_sub(500),
                ..andando
            },
            0,
        )
        .posicao;
    assert_eq!(antes, depois);
}

/// Um pacote com o mesmo relógio é o mesmo pacote: devolve o mesmo cálculo, e a média não cresce com as repetições.
#[test]
fn pacote_repetido_nao_entra_de_novo_na_media() {
    let (mut e, p) = nova();
    let r = rpm_de(0.4, &p);
    let primeiro = e.passo(
        &Medidas {
            rodas_rpm: [r; 4],
            ..parado_em_pe()
        },
        0,
    );
    let mut ultimo = primeiro;
    for _ in 0..100 {
        ultimo = e.passo(&parado_em_pe(), 0);
    }
    assert_eq!(ultimo, primeiro, "o relógio não andou: é o mesmo pacote");
}

/// O tempo das linhas do reprocessamento segue a regra da lacuna: um relógio que recomeça não salta 49 dias.
#[test]
fn relogio_que_recomeca_nao_salta_o_tempo_das_linhas() {
    use rms1_control::estimativa::reprocessar;
    use rms1_proto::barramento::Layout;
    use rms1_proto::{PedeAck, Quadro};
    let quadro = |relogio_ms: u32| {
        let mut d = vec![0u8; 163];
        d[1] = 1;
        d[2..6].copy_from_slice(&relogio_ms.to_le_bytes());
        Quadro {
            origem: 0x03,
            destino: 0x09,
            seq: 0,
            resposta: false,
            pede: PedeAck::Agora,
            cmd_set: 0x48,
            cmd_id: 0x08,
            dados: d,
        }
    };
    let qs: Vec<Quadro> = [1000, 1020, 5, 25].into_iter().map(quadro).collect();
    let l = reprocessar(&qs, &Layout::da_assinatura(), &Params::default());
    let t: Vec<f64> = l.iter().map(|x| x.t).collect();
    assert_eq!(t, vec![0.0, 0.02, 0.02, 0.04]);
}

/// Só a nossa mensagem, para o nosso nó: o pacote de outra assinatura não entra no reprocessamento do `strace`.
#[test]
fn reprocessar_ignora_outra_mensagem_e_outro_no() {
    use rms1_control::estimativa::reprocessar;
    use rms1_proto::barramento::Layout;
    use rms1_proto::{PedeAck, Quadro};
    let quadro = |mensagem: u8, destino: u8| {
        let mut d = vec![0u8; 163];
        d[1] = mensagem;
        Quadro {
            origem: 0x03,
            destino,
            seq: 0,
            resposta: false,
            pede: PedeAck::Agora,
            cmd_set: 0x48,
            cmd_id: 0x08,
            dados: d,
        }
    };
    let qs = vec![quadro(1, 0x09), quadro(2, 0x09), quadro(1, 0x0a)];
    assert_eq!(
        reprocessar(&qs, &Layout::da_assinatura(), &Params::default()).len(),
        1
    );
}
