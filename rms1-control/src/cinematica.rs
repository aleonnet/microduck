//! Mecanum com roletes a 45°, de giro do corpo para rpm de roda.

/// Pedido no corpo (x à frente, y à esquerda, giro à esquerda em rad/s) para rpm, com sinal de andar
/// para a frente, na ordem frente-direita, frente-esquerda, trás-esquerda, trás-direita.
///
/// Sem limite aqui: quem limita é a segurança.
pub fn rpm_de_twist(twist: [f64; 3], raio_m: f64, meia_soma_m: f64) -> [f64; 4] {
    let [vx, vy, giro] = twist;
    let k = meia_soma_m * giro;
    let rpm = |v: f64| v / raio_m * 60.0 / std::f64::consts::TAU;
    [
        rpm(vx + vy + k),
        rpm(vx - vy - k),
        rpm(vx + vy - k),
        rpm(vx - vy + k),
    ]
}

/// A inversa de [`rpm_de_twist`]: das quatro rodas, na mesma ordem e no mesmo sinal, para o giro do corpo. Quatro rodas
/// para três incógnitas: a sobra (roda que escorrega ou mede errado) sai no mínimo erro quadrático, que aqui é a média.
pub fn twist_de_rpm(rpm: [f64; 4], raio_m: f64, meia_soma_m: f64) -> [f64; 3] {
    let v = rpm.map(|r| r * std::f64::consts::TAU / 60.0 * raio_m);
    let [fr, fl, bl, br] = v;
    [
        (fr + fl + bl + br) / 4.0,
        (fr - fl + bl - br) / 4.0,
        (fr - fl - bl + br) / 4.0 / meia_soma_m,
    ]
}

#[cfg(test)]
mod testes {
    use super::*;

    fn r(twist: [f64; 3]) -> [f64; 4] {
        rpm_de_twist(twist, 0.05, 0.2)
    }

    /// Andar reto, de lado, girar no lugar e tudo junto: a inversa devolve o pedido.
    #[test]
    fn ida_e_volta_devolve_o_pedido() {
        for t in [
            [1.0, 0.0, 0.0],
            [0.0, -0.7, 0.0],
            [0.0, 0.0, 2.5],
            [0.3, 0.2, -1.1],
        ] {
            let v = twist_de_rpm(r(t), 0.05, 0.2);
            for i in 0..3 {
                assert!((v[i] - t[i]).abs() < 1e-9, "{t:?} -> {v:?}");
            }
        }
    }

    #[test]
    fn para_a_frente_as_quatro_giram_iguais_e_positivas() {
        let [fr, fl, bl, br] = r([1.0, 0.0, 0.0]);
        let esperado = 1.0 / 0.05 * 60.0 / std::f64::consts::TAU;
        for v in [fr, fl, bl, br] {
            assert!((v - esperado).abs() < 1e-9, "{v}");
        }
    }

    #[test]
    fn para_a_esquerda_as_diagonais_se_opoem() {
        let [fr, fl, bl, br] = r([0.0, 1.0, 0.0]);
        assert!(fr > 0.0 && bl > 0.0 && fl < 0.0 && br < 0.0);
        assert_eq!(fr, bl);
        assert_eq!(fl, br);
    }

    #[test]
    fn girando_a_esquerda_o_lado_direito_anda_para_a_frente() {
        let [fr, fl, bl, br] = r([0.0, 0.0, 1.0]);
        assert!(fr > 0.0 && br > 0.0 && fl < 0.0 && bl < 0.0);
    }
}
