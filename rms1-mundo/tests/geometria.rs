//! A geometria: um ponto do quadro levado ao chão e de volta, com a torre girada e inclinada e o corpo em qualquer pose, e os
//! casos de valor conhecido, feitos à mão.

mod comum;

use rms1_mundo::geometria::{corpo_para_odom, odom_para_corpo};
use rms1_mundo::{Olho, Raios, Vista};
use rms1d_params::{Camera, CameraNoCorpo};

/// Uma câmera de números redondos: o foco de 500 pontos, o centro no meio de 1280 por 720, o eixo da torre a 20 cm do chão, e
/// a lente 8 cm à frente e 3 cm acima dele.
fn redonda() -> Olho {
    Olho {
        lente: Camera {
            fx: 500.0,
            fy: 500.0,
            cx: 640.0,
            cy: 360.0,
            largura: 1280.0,
            altura: 720.0,
            zero_inclinacao_grau: 0.0,
            atraso_ms: 0.0,
        },
        no_corpo: CameraNoCorpo {
            eixo_altura_m: 0.2,
            lente_frente_m: 0.08,
            lente_acima_m: 0.03,
        },
    }
}

fn perto(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn com_a_torre_nivelada_o_chao_fica_onde_a_conta_a_mao_diz() {
    let olho = redonda();
    let vista = Vista::nova(&olho, &comum::amostra(0.0, 0.0, 0.0, [0.0; 3]));
    assert!(
        perto(vista.altura, 0.23),
        "a lente fica a 20 + 3 cm do chão: {}",
        vista.altura
    );
    // 250 pontos abaixo do centro, com o foco de 500: o raio desce meio metro por metro, e toca o chão a 0,46 m da lente.
    let [xn, yn] = olho.normalizado(640.0, 610.0);
    assert!(perto(xn, 0.0) && perto(yn, 0.5));
    let z = vista.chao(yn).unwrap();
    assert!(perto(z, 0.46), "{z}");
    let p = vista.no_corpo(xn, yn, z);
    assert!(
        perto(p[0], 0.54) && perto(p[1], 0.0) && perto(p[2], 0.0),
        "a 8 + 46 cm do eixo, no chão: {p:?}"
    );
    // O mesmo ponto 250 pontos à direita do centro: 23 cm para a direita, que é y negativo.
    let [xn, yn] = olho.normalizado(890.0, 610.0);
    let p = vista.no_corpo(xn, yn, z);
    assert!(
        perto(p[0], 0.54) && perto(p[1], -0.23) && perto(p[2], 0.0),
        "{p:?}"
    );
    // A meio caminho do chão, o ponto está a meia altura da lente.
    let p = vista.no_corpo(0.0, 0.5, 0.23);
    assert!(perto(p[2], 0.115), "{p:?}");
    // Do horizonte para cima não há chão.
    assert!(vista.chao(0.0).is_none() && vista.chao(-0.2).is_none());
}

#[test]
fn com_a_torre_inclinada_a_lente_muda_de_altura_e_o_chao_chega_mais_perto() {
    let olho = redonda();
    let vista = Vista::nova(&olho, &comum::amostra(0.0, 0.0, -30.0, [0.0; 3]));
    // A lente gira em volta do eixo: desce 8 cm × sen 30° e os 3 cm de cima viram 3 × cos 30°.
    let altura = 0.2 - 0.08 * 0.5 + 0.03 * 30f64.to_radians().cos();
    assert!(perto(vista.altura, altura), "{} e {altura}", vista.altura);
    // O centro do quadro olha 30° para baixo: toca o chão a altura / tan 30° da lente, que está à frente do eixo
    // 8 × cos 30° + 3 × sen 30° centímetros.
    let p = vista.no_corpo(0.0, 0.0, vista.chao(0.0).unwrap());
    let a_frente = 0.08 * 30f64.to_radians().cos() + 0.03 * 0.5;
    assert!(
        perto(p[0], a_frente + altura / 30f64.to_radians().tan()) && perto(p[2], 0.0),
        "{p:?}"
    );
    // Olhando 10° para cima, a linha do centro fica acima do horizonte, e o horizonte desce no quadro.
    let para_cima = Vista::nova(&olho, &comum::amostra(0.0, 0.0, 10.0, [0.0; 3]));
    assert!(para_cima.chao(0.0).is_none());
    assert!(para_cima.chao(10f64.to_radians().tan() + 1e-6).is_some());
}

#[test]
fn o_zero_da_inclinacao_soma_ao_que_a_torre_diz() {
    let mut olho = redonda();
    olho.lente.zero_inclinacao_grau = -4.0;
    let com_zero = Vista::nova(&olho, &comum::amostra(0.0, 0.0, 1.0, [0.0; 3]));
    let sem_zero = Vista::nova(&redonda(), &comum::amostra(0.0, 0.0, -3.0, [0.0; 3]));
    assert_eq!(com_zero, sem_zero);
}

#[test]
fn o_giro_da_torre_e_a_pose_do_corpo_levam_o_ponto_ao_lugar() {
    let olho = redonda();
    // A torre 90° à esquerda: o que estava à frente fica à esquerda do corpo.
    let vista = Vista::nova(&olho, &comum::amostra(0.0, 90.0, 0.0, [0.0; 3]));
    let p = vista.no_corpo(0.0, 0.5, 0.46);
    assert!(perto(p[0], 0.0) && perto(p[1], 0.54), "{p:?}");
    // O corpo em (1, 2), virado 90° à esquerda, com a torre de frente: o ponto vai para (1, 2,54) na odometria.
    let vista = Vista::nova(&olho, &comum::amostra(0.0, 0.0, 0.0, [1.0, 2.0, 90.0]));
    let p = vista.no_odom(0.0, 0.5, 0.46);
    assert!(
        perto(p[0], 1.0) && perto(p[1], 2.54) && perto(p[2], 0.0),
        "{p:?}"
    );
    // E os dois juntos: corpo a 90°, torre a -90°, a câmera olha para onde olhava no começo.
    let vista = Vista::nova(&olho, &comum::amostra(0.0, -90.0, 0.0, [1.0, 2.0, 90.0]));
    let p = vista.no_odom(0.0, 0.5, 0.46);
    assert!(perto(p[0], 1.54) && perto(p[1], 2.0), "{p:?}");
}

#[test]
fn a_ida_e_a_volta_fecham_em_qualquer_arranjo() {
    for olho in [
        redonda(),
        comum::olho_do_simulador(),
        comum::olho_de_teste(),
    ] {
        for (giro, inclinacao, pose) in [
            (0.0, 0.0, [0.0, 0.0, 0.0]),
            (37.0, -20.0, [1.5, -0.7, 130.0]),
            (-140.0, 12.0, [-3.0, 2.0, -75.0]),
            (250.0, 35.0, [0.2, 0.1, 10.0]),
        ] {
            let a = comum::amostra(0.0, giro, inclinacao, pose);
            let vista = Vista::nova(&olho, &a);
            for (xn, yn, z) in [
                (0.0, 0.0, 1.0),
                (-0.8, 0.4, 0.37),
                (1.1, -0.3, 2.9),
                (0.05, 0.9, 0.2),
            ] {
                let p = vista.no_corpo(xn, yn, z);
                let [xv, yv, zv] = vista.da_lente(p).expect("o ponto está à frente da lente");
                assert!(
                    perto(xv, xn) && perto(yv, yn) && perto(zv, z),
                    "a ida e a volta: ({xn}, {yn}, {z}) voltou ({xv}, {yv}, {zv})"
                );
                let odom = corpo_para_odom(a.pose, [p[0], p[1]]);
                let de_volta = odom_para_corpo(a.pose, odom);
                assert!(perto(de_volta[0], p[0]) && perto(de_volta[1], p[1]));
                let direto = vista.no_odom(xn, yn, z);
                assert!(
                    perto(direto[0], odom[0])
                        && perto(direto[1], odom[1])
                        && perto(direto[2], p[2])
                );
            }
            // O que fica atrás da lente não aparece no quadro.
            assert!(vista.da_lente(vista.no_corpo(0.0, 0.0, -1.0)).is_none());
        }
    }
}

#[test]
fn a_grade_cobre_o_quadro_inteiro_da_lente() {
    let olho = redonda();
    let raios = Raios::da_grade(&olho, 320, 180);
    assert!(raios.serve(320, 180) && !raios.serve(224, 126));
    // O meio da primeira coluna da grade fica a dois pontos da borda do quadro de 1280, e o da última, a dois da outra.
    assert!(
        perto(raios.xn[0], (2.0 - 640.0) / 500.0) && perto(raios.xn[319], (1278.0 - 640.0) / 500.0)
    );
    assert!(
        perto(raios.yn[0], (2.0 - 360.0) / 500.0) && perto(raios.yn[179], (718.0 - 360.0) / 500.0)
    );
    let [u, v] = olho.no_quadro(raios.xn[100], raios.yn[50]);
    assert!(perto(u, 402.0) && perto(v, 202.0), "({u}, {v})");
    // A lente do simulador: 120° na horizontal, e 2 × atan(360 / foco) na vertical.
    let [h, v] = comum::olho_do_simulador().campo_grau();
    assert!(
        (h - 120.0).abs() < 1e-9 && (v - 88.50).abs() < 0.01,
        "{h} por {v}"
    );
}
