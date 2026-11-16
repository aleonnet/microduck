//! O quadro que vai à estação: o NV12 da câmera reduzido a RGB, sem moldura.
//!
//! A conta é a do `tools/hailo/prepara_s1.c` do laboratório, o programa da medição de 2026-11-13, sem as faixas da moldura:
//! vizinho mais próximo, e BT.601 de faixa limitada (o que o driver da câmera declara), em inteiros. A moldura de cada
//! modelo é posta na estação. `scripts/test-rms1-mediad-lab.sh` prova que as duas contas, juntas, dão os mesmos bytes.
//!
//! O quadro do robô, 1280 por 720, vai à estação reduzido a 640 por 360 (`percepcao.rs`, `QUADRO_LARGURA` e
//! `QUADRO_ALTURA`): 691.200 bytes. A medição de 2026-11-13 usava 320 por 180, e é esse o tamanho que os testes daqui conferem.

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

/// Reduz um quadro NV12 de `l` por `a` a NV12 de `ls` por `as_`, pelo vizinho mais próximo, sem passar por RGB: é o que o
/// codificador de hardware do S1 recebe. A croma do quadro reduzido é a do pixel de cada par, como em [`nv12_para_rgb`], e
/// fica com a metade da resolução do quadro reduzido, que é o que o NV12 guarda. `None` como lá.
pub fn nv12_reduzido(nv12: &[u8], l: usize, a: usize, ls: usize, as_: usize) -> Option<Vec<u8>> {
    if l == 0
        || a == 0
        || ls == 0
        || as_ == 0
        || !ls.is_multiple_of(2)
        || !as_.is_multiple_of(2)
        || nv12.len() != l * a * 3 / 2
    {
        return None;
    }
    let (luma, croma) = nv12.split_at(l * a);
    let mut saida = Vec::with_capacity(ls * as_ * 3 / 2);
    for y in 0..as_ {
        let sy = y * a / as_;
        saida.extend((0..ls).map(|x| luma[sy * l + x * l / ls]));
    }
    for y in (0..as_).step_by(2) {
        let sy = y * a / as_;
        for x in (0..ls).step_by(2) {
            let par = (sy / 2) * l + ((x * l / ls) & !1);
            saida.extend([croma[par], croma[par + 1]]);
        }
    }
    Some(saida)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn o_nv12_reduzido_guarda_a_luma_e_a_croma_do_vizinho_mais_proximo() {
        // 8 por 4 para 4 por 2: a luma de cada segundo pixel de cada segunda linha, e um par de croma por bloco de 2 por 2.
        let luma: Vec<u8> = (0..32).collect();
        let croma: Vec<u8> = (100..116).collect();
        let nv12 = [luma, croma].concat();
        let r = nv12_reduzido(&nv12, 8, 4, 4, 2).unwrap();
        assert_eq!(r.len(), 4 * 2 * 3 / 2);
        assert_eq!(&r[..8], &[0, 2, 4, 6, 16, 18, 20, 22]);
        // A linha de croma 0 do quadro de origem, nos pares das colunas 0 e 4.
        assert_eq!(&r[8..], &[100, 101, 104, 105]);
        // No mesmo tamanho, sai o mesmo quadro.
        assert_eq!(nv12_reduzido(&nv12, 8, 4, 8, 4).unwrap(), nv12);
        // Tamanho errado, ou saída de lado ímpar, não tem resposta.
        assert!(nv12_reduzido(&nv12[1..], 8, 4, 4, 2).is_none());
        assert!(nv12_reduzido(&nv12, 8, 4, 3, 2).is_none());
    }

    #[test]
    fn o_quadro_reduzido_em_nv12_da_o_mesmo_rgb_onde_a_croma_e_lisa() {
        // Com a croma igual no quadro todo, reduzir em NV12 e converter dá o mesmo que converter reduzindo: a diferença
        // entre os dois caminhos é só a resolução da croma.
        let mut nv12: Vec<u8> = (0..64u32).map(|i| (i * 3 + 20) as u8).collect();
        nv12.extend(std::iter::repeat_n([90u8, 200u8], 16).flatten());
        let direto = nv12_para_rgb(&nv12, 8, 8, 4, 4).unwrap();
        let pequeno = nv12_reduzido(&nv12, 8, 8, 4, 4).unwrap();
        assert_eq!(nv12_para_rgb(&pequeno, 4, 4, 4, 4).unwrap(), direto);
    }

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
