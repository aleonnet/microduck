//! O quadro que vai à estação: o NV12 da câmera reduzido a RGB, sem moldura.
//!
//! A conta é a do `tools/hailo/prepara_s1.c` do laboratório, o programa da medição de 2026-11-13, sem as faixas da moldura:
//! vizinho mais próximo, e BT.601 de faixa limitada (o que o driver da câmera declara), em inteiros. A moldura de cada
//! modelo é posta na estação. `scripts/test-rms1-mediad-lab.sh` prova que as duas contas, juntas, dão os mesmos bytes.
//!
//! Para o quadro do robô, 1280 por 720 reduzido a 320 por 180: 172.800 bytes, contra os 307.200 do quadro com moldura.

fn saturar(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Reduz um quadro NV12 de `l` por `a` a RGB de `ls` por `as_`. `None` se o quadro não tem o tamanho que diz ter, ou se
/// algum tamanho é zero.
pub fn nv12_para_rgb(nv12: &[u8], l: usize, a: usize, ls: usize, as_: usize) -> Option<Vec<u8>> {
    if l == 0 || a == 0 || ls == 0 || as_ == 0 || nv12.len() != l * a * 3 / 2 {
        return None;
    }
    let (luma, croma) = nv12.split_at(l * a);
    let mut rgb = Vec::with_capacity(ls * as_ * 3);
    for y in 0..as_ {
        let sy = y * a / as_;
        for x in 0..ls {
            let sx = x * l / ls;
            let yy = i32::from(luma[sy * l + sx]) - 16;
            let par = (sy / 2) * l + (sx & !1);
            let u = i32::from(croma[par]) - 128;
            let v = i32::from(croma[par + 1]) - 128;
            rgb.push(saturar((298 * yy + 409 * v + 128) >> 8));
            rgb.push(saturar((298 * yy - 100 * u - 208 * v + 128) >> 8));
            rgb.push(saturar((298 * yy + 516 * u + 128) >> 8));
        }
    }
    Some(rgb)
}

#[cfg(test)]
mod testes {
    use super::*;

    /// Um quadro NV12 de uma cor só.
    fn liso(l: usize, a: usize, y: u8, u: u8, v: u8) -> Vec<u8> {
        let mut q = vec![y; l * a];
        for _ in 0..l * a / 4 {
            q.extend([u, v]);
        }
        q
    }

    #[test]
    fn as_cores_de_referencia_do_bt601_saem_certas() {
        // Preto, branco e cinza da faixa limitada; e o vermelho, que só sai vermelho com U e V no lugar certo.
        for (yuv, rgb) in [
            ((16, 128, 128), [0, 0, 0]),
            ((235, 128, 128), [255, 255, 255]),
            ((126, 128, 128), [128, 128, 128]),
            ((81, 90, 240), [255, 0, 0]),
            ((41, 240, 110), [0, 0, 255]),
        ] {
            let saida = nv12_para_rgb(&liso(8, 4, yuv.0, yuv.1, yuv.2), 8, 4, 2, 2).unwrap();
            assert_eq!(saida, rgb.repeat(4), "{yuv:?}");
        }
    }

    #[test]
    fn o_tamanho_do_quadro_do_robo_e_o_tamanho_errado() {
        let q = liso(1280, 720, 126, 128, 128);
        assert_eq!(
            nv12_para_rgb(&q, 1280, 720, 320, 180).unwrap().len(),
            172_800
        );
        assert!(
            nv12_para_rgb(&q[1..], 1280, 720, 320, 180).is_none(),
            "um byte a menos"
        );
        assert!(nv12_para_rgb(&q, 1280, 720, 0, 180).is_none());
    }

    #[test]
    fn cada_pixel_vem_do_lugar_certo() {
        // Luma que cresce com a coluna: a redução por 4 pega as colunas 0, 4, 8 e 12.
        let (l, a) = (16, 4);
        let mut q: Vec<u8> = (0..a)
            .flat_map(|_| (0..l).map(|x| 16 + 10 * x as u8))
            .collect();
        q.extend(vec![128u8; l * a / 2]);
        let saida = nv12_para_rgb(&q, l, a, 4, 1).unwrap();
        let cinza = |y: i32| saturar((298 * (y - 16) + 128) >> 8);
        assert_eq!(
            saida,
            [0, 4, 8, 12]
                .iter()
                .flat_map(|&x| [cinza(16 + 10 * x); 3])
                .collect::<Vec<_>>()
        );
    }
}
