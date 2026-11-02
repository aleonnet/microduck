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

#[cfg(test)]
mod testes {
    use super::*;

    fn r(twist: [f64; 3]) -> [f64; 4] {
        rpm_de_twist(twist, 0.05, 0.2)
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
