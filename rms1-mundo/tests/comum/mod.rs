//! A cena de mentira dos testes: um chão, caixas de lados retos, e a grade que o `scdepthv3` daria dela.
//!
//! A conta daqui é feita por extenso, com vetores no espaço, e não com as funções do pacote: a grade que entra nos testes não
//! pode depender do que eles provam. O raio de cada ponto sai da lente girada pela inclinação e pelo giro da torre e pelo rumo
//! do corpo, e bate no que estiver mais perto, o chão ou uma caixa.
#![allow(dead_code)]

use rms1_mundo::{Ajustes, Amostra, Corpo, Mundo, Olho};
use rms1d_params::{Camera, CameraNoCorpo, Fisica};

/// A grade do `scdepthv3`, como a estação a entrega: 320 por 180, em 16 bits, com a quantização medida em 2026-11-20.
pub const LARGURA: usize = 320;
pub const ALTURA: usize = 180;
pub const ESCALA_Q: f32 = 0.000_263_199_68;
pub const ZERO_Q: f32 = 32767.0;

/// A câmera do simulador, que é exata por construção.
pub fn olho_do_simulador() -> Olho {
    let f = Fisica::da_tabela();
    Olho {
        lente: f.camera_simulada,
        no_corpo: f.camera_no_corpo,
        cano: None,
    }
}

/// A câmera do robô, como ele a tem hoje: a lente provisória, do arquivo de fábrica (aqui com números de teste, na forma
/// dele: os do arquivo são de cada robô), com o cano do lançador no quadro.
pub fn olho_do_robo() -> Olho {
    let f = Fisica::da_tabela();
    Olho {
        lente: f
            .camera_pela_de_fabrica("700.5,481.25,349.5,0.000000")
            .expect("a lente de fábrica de teste"),
        no_corpo: f.camera_no_corpo,
        cano: Some(f.cano_na_lente),
    }
}

/// Põe o cano do lançador nas profundidades de uma cena: onde o raio de um ponto da grade entra no cilindro dele, a
/// profundidade passa a ser a dessa entrada. Por amostragem ao longo do raio, e não pela conta fechada do pacote: o cano anda
/// com a lente, e por isso a conta é feita na lente, sem a torre nem o corpo.
pub fn por_o_cano(olho: &Olho, profundidades: &mut [f64]) {
    let cano = olho.cano.expect("esta câmera vê o cano");
    let l = &olho.lente;
    for j in 0..ALTURA {
        for i in 0..LARGURA {
            let u = (i as f64 + 0.5) * l.largura / LARGURA as f64;
            let v = (j as f64 + 0.5) * l.altura / ALTURA as f64;
            let (xn, yn) = ((u - l.cx) / l.fx, (v - l.cy) / l.fy);
            for passo in 1..=400 {
                let t = cano.frente_m * f64::from(passo) / 400.0;
                let (esquerda, cima) = (-t * xn, -t * yn);
                let fora_do_eixo =
                    (esquerda - cano.esquerda_m).hypot(cima + cano.abaixo_m);
                if fora_do_eixo <= cano.raio_m {
                    let k = j * LARGURA + i;
                    profundidades[k] = profundidades[k].min(t);
                    break;
                }
            }
        }
    }
}

/// Uma câmera de teste: mais fechada que a do simulador, com o centro fora do meio do quadro e o zero da inclinação fora de
/// zero, para os testes passarem por cada termo da conta. Não é a do robô, que só existe medida.
pub fn olho_de_teste() -> Olho {
    Olho {
        lente: Camera {
            fx: 700.0,
            fy: 690.0,
            cx: 652.0,
            cy: 348.0,
            largura: 1280.0,
            altura: 720.0,
            zero_inclinacao_grau: 1.5,
            atraso_ms: 0.0,
            provisoria: false,
        },
        no_corpo: CameraNoCorpo {
            eixo_altura_m: 0.21,
            lente_frente_m: 0.07,
            lente_acima_m: 0.02,
        },
        cano: None,
    }
}

pub fn corpo() -> Corpo {
    let f = Fisica::da_tabela();
    Corpo {
        largura_m: f.largura_m,
        frente_m: f.frente_m,
    }
}

pub fn ajustes() -> Ajustes {
    Ajustes::default()
}

pub fn mundo(olho: Olho) -> Mundo {
    Mundo::novo(Some(olho), corpo(), ajustes()).unwrap()
}

/// Uma amostra, com os ângulos em graus.
pub fn amostra(t: f64, giro_grau: f64, inclinacao_grau: f64, pose: [f64; 3]) -> Amostra {
    Amostra {
        t,
        torre: [giro_grau.to_radians(), inclinacao_grau.to_radians()],
        pose: [pose[0], pose[1], pose[2].to_radians()],
    }
}

/// Uma caixa de lados retos, na odometria: de `x[0]` a `x[1]`, de `y[0]` a `y[1]`, do chão até `altura`.
#[derive(Debug, Clone, Copy)]
pub struct Caixa {
    pub x: [f64; 2],
    pub y: [f64; 2],
    pub altura: f64,
}

/// Uma caixa de 30 cm de lado e 25 cm de altura, com a face de perto em `x` e o meio em `y`.
pub fn caixa_em(x: f64, y: f64) -> Caixa {
    Caixa {
        x: [x, x + 0.3],
        y: [y - 0.15, y + 0.15],
        altura: 0.25,
    }
}

/// Uma parede alta, de lado a lado, com a face em `x`.
pub fn parede_em(x: f64) -> Caixa {
    Caixa {
        x: [x, x + 0.2],
        y: [-20.0, 20.0],
        altura: 3.0,
    }
}

type V3 = [f64; 3];

/// A lente no espaço (a odometria, com z para cima) e o raio do ponto `(i, j)` da grade, com a profundidade valendo 1.
fn raio(olho: &Olho, a: &Amostra, i: usize, j: usize) -> (V3, V3) {
    let l = &olho.lente;
    let u = (i as f64 + 0.5) * l.largura / LARGURA as f64;
    let v = (j as f64 + 0.5) * l.altura / ALTURA as f64;
    let (xn, yn) = ((u - l.cx) / l.fx, (v - l.cy) / l.fy);
    let inclinacao = a.torre[1] + l.zero_inclinacao_grau.to_radians();
    let rumo = a.torre[0] + a.pose[2];
    // Um vetor da lente (frente, esquerda, cima), girado pela inclinação em volta do eixo da esquerda, e depois pelo rumo.
    let girar = |p: V3| -> V3 {
        let frente = p[0] * inclinacao.cos() - p[2] * inclinacao.sin();
        let cima = p[0] * inclinacao.sin() + p[2] * inclinacao.cos();
        [
            frente * rumo.cos() - p[1] * rumo.sin(),
            frente * rumo.sin() + p[1] * rumo.cos(),
            cima,
        ]
    };
    let c = &olho.no_corpo;
    let da_lente = girar([c.lente_frente_m, 0.0, c.lente_acima_m]);
    let origem = [
        a.pose[0] + da_lente[0],
        a.pose[1] + da_lente[1],
        c.eixo_altura_m + da_lente[2],
    ];
    (origem, girar([1.0, -xn, -yn]))
}

/// Onde o raio entra na caixa, em profundidades; nada, se não a toca.
fn na_caixa(o: V3, d: V3, c: &Caixa) -> Option<f64> {
    let (mut perto, mut longe) = (0.0f64, f64::INFINITY);
    for (k, [de, ate]) in [c.x, c.y, [0.0, c.altura]].into_iter().enumerate() {
        if d[k].abs() < 1e-12 {
            if o[k] < de || o[k] > ate {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((de - o[k]) / d[k], (ate - o[k]) / d[k]);
        perto = perto.max(t0.min(t1));
        longe = longe.min(t0.max(t1));
    }
    (perto <= longe && perto > 0.0).then_some(perto)
}

/// A profundidade de verdade de cada ponto da grade: o infinito onde o raio não bate em nada.
pub fn profundidades(olho: &Olho, a: &Amostra, caixas: &[Caixa]) -> Vec<f64> {
    let mut saida = Vec::with_capacity(LARGURA * ALTURA);
    for j in 0..ALTURA {
        for i in 0..LARGURA {
            let (o, d) = raio(olho, a, i, j);
            let mut z = if d[2] < 0.0 {
                -o[2] / d[2]
            } else {
                f64::INFINITY
            };
            for c in caixas {
                if let Some(t) = na_caixa(o, d, c) {
                    z = z.min(t);
                }
            }
            saida.push(z);
        }
    }
    saida
}

/// A entrega da estação para estas profundidades: o modelo dá o inverso da profundidade vezes `k`, a escala que ninguém
/// conhece, e a estação entrega o número de antes da conta `10 * sigmoide(x) + 0,009`, em 16 bits. `ruido` é a semente e a
/// fração do ruído, ponto a ponto.
pub fn entrega(profundidades: &[f64], k: f64, ruido: Option<(u64, f64)>) -> Vec<u8> {
    let mut carga = Vec::with_capacity(16 + profundidades.len() * 2);
    carga.extend_from_slice(&(LARGURA as u16).to_le_bytes());
    carga.extend_from_slice(&(ALTURA as u16).to_le_bytes());
    carga.extend_from_slice(&[1, 3, 0, 0]);
    carga.extend_from_slice(&ESCALA_Q.to_le_bytes());
    carga.extend_from_slice(&ZERO_Q.to_le_bytes());
    let mut estado = ruido.map_or(0, |(semente, _)| semente);
    for z in profundidades {
        let mut u = if z.is_finite() { k / z } else { 0.0 };
        if let Some((_, fracao)) = ruido {
            // Um gerador congruente, o de Knuth: o mesmo ruído a cada rodada.
            estado = estado
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let sorteio = (estado >> 11) as f64 / (1u64 << 53) as f64;
            u *= 1.0 + fracao * (2.0 * sorteio - 1.0);
        }
        // A conta ao contrário: s = (u - 0,009) / 10, x = ln(s / (1 - s)), q = x / escala + zero.
        let s = ((u - 0.009) / 10.0).clamp(1e-7, 1.0 - 1e-7);
        let x = (s / (1.0 - s)).ln();
        let q = (x / f64::from(ESCALA_Q) + f64::from(ZERO_Q))
            .round()
            .clamp(0.0, 65535.0) as u16;
        carga.extend_from_slice(&q.to_le_bytes());
    }
    carga
}

/// A cena inteira num passo: a entrega do que a câmera vê, posta em `a`.
pub fn ver(olho: &Olho, a: &Amostra, caixas: &[Caixa], k: f64) -> Vec<u8> {
    entrega(&profundidades(olho, a, caixas), k, None)
}

/// Um número com vírgula, como se escreve.
pub fn br(v: f64) -> String {
    format!("{v:.2}").replace('.', ",")
}
