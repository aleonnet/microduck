//! Mede quanto custa preparar o quadro que vai à estação, com as mesmas funções que o `rms1-mediad` usa: reduzir o NV12 da
//! câmera a RGB (`reducao::nv12_para_rgb`) e codificar em JPEG (`servidor::jpeg_de_rgb`). O mesmo programa roda no Mac e no
//! processador do S1, e os JPEG que ele grava são os que a comparação manda ao detector (adendo 4 do plano, 2026-11-15).
//!
//! Uso: rms1-mediad-codifica <quadro.nv12> <largura> <altura> <largura de saída> <altura de saída> <voltas>
//!                           <qualidade>[,<qualidade>...] [<pasta de saída>]
//! Imprime, por etapa, a mediana e o pior tempo de um quadro, em ms, e o tamanho do resultado. Com a pasta, grava o RGB
//! reduzido (`quadro.rgb`) e um `q<qualidade>.jpg` de cada qualidade.

use std::time::Instant;

use rms1_mediad::reducao::{nv12_para_rgb, nv12_reduzido};
use rms1_mediad::servidor::jpeg_de_rgb;

/// A mediana e o maior de uma lista de tempos, em ms.
fn resumo(mut ms: Vec<f64>) -> (f64, f64) {
    ms.sort_by(|a, b| a.total_cmp(b));
    (ms[ms.len() / 2], ms[ms.len() - 1])
}

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    anyhow::ensure!(
        a.len() >= 8,
        "uso: {} <quadro.nv12> <largura> <altura> <largura de saída> <altura de saída> <voltas> <qualidades> [<pasta>]",
        a[0]
    );
    let nv12 = std::fs::read(&a[1])?;
    let (l, alt): (usize, usize) = (a[2].parse()?, a[3].parse()?);
    let (ls, as_): (usize, usize) = (a[4].parse()?, a[5].parse()?);
    let voltas: usize = a[6].parse()?;
    anyhow::ensure!(voltas > 0, "pelo menos uma volta");
    anyhow::ensure!(
        nv12.len() >= l * alt * 3 / 2,
        "o arquivo é menor que um quadro NV12 de {l} por {alt}"
    );
    // Uma gravação do driver traz 32 bytes a mais no fim: só o quadro entra na conta.
    let nv12 = &nv12[..l * alt * 3 / 2];
    let qualidades: Vec<u8> = a[7].split(',').map(str::parse).collect::<Result<_, _>>()?;
    let pasta = a.get(8);

    // O que a percepção faz a cada quadro quando o quadro vai codificado pelo chip: reduzir o NV12 a NV12. E, para comparar,
    // o que a câmera faz: copiar o quadro inteiro para uma memória nova.
    let mut tempos = Vec::with_capacity(voltas);
    let mut pequeno = Vec::new();
    for _ in 0..voltas {
        let t = Instant::now();
        pequeno = nv12_reduzido(nv12, l, alt, ls, as_)
            .ok_or_else(|| anyhow::anyhow!("a redução a NV12 recusou o quadro"))?;
        tempos.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let (mediana, pior) = resumo(tempos);
    println!(
        "reduzir {l}x{alt} NV12 a {ls}x{as_} NV12: mediana {mediana:.2} ms, pior {pior:.2} ms, {} bytes",
        pequeno.len()
    );
    let mut tempos = Vec::with_capacity(voltas);
    for _ in 0..voltas {
        let t = Instant::now();
        let copia = nv12.to_vec();
        tempos.push(t.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(&copia);
    }
    let (mediana, pior) = resumo(tempos);
    println!(
        "copiar o quadro de {l}x{alt} para memória nova: mediana {mediana:.2} ms, pior {pior:.2} ms, {} bytes",
        nv12.len()
    );
    let mut reuso = vec![0u8; nv12.len()];
    let mut tempos = Vec::with_capacity(voltas);
    for _ in 0..voltas {
        let t = Instant::now();
        reuso.copy_from_slice(nv12);
        tempos.push(t.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(&reuso);
    }
    let (mediana, pior) = resumo(tempos);
    println!("copiar o quadro para a mesma memória: mediana {mediana:.2} ms, pior {pior:.2} ms");

    let mut tempos = Vec::with_capacity(voltas);
    let mut rgb = Vec::new();
    for _ in 0..voltas {
        let t = Instant::now();
        rgb = nv12_para_rgb(nv12, l, alt, ls, as_)
            .ok_or_else(|| anyhow::anyhow!("a redução recusou o quadro"))?;
        tempos.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let (mediana, pior) = resumo(tempos);
    println!(
        "reduzir {l}x{alt} NV12 a {ls}x{as_} RGB: mediana {mediana:.2} ms, pior {pior:.2} ms, {} bytes",
        rgb.len()
    );
    if let Some(p) = pasta {
        std::fs::create_dir_all(p)?;
        std::fs::write(format!("{p}/quadro.rgb"), &rgb)?;
    }
    for q in qualidades {
        let mut tempos = Vec::with_capacity(voltas);
        let mut jpeg = Vec::new();
        for _ in 0..voltas {
            let t = Instant::now();
            jpeg = jpeg_de_rgb(&rgb, ls as u32, as_ as u32, q)
                .ok_or_else(|| anyhow::anyhow!("o JPEG não saiu"))?;
            tempos.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        let (mediana, pior) = resumo(tempos);
        println!(
            "JPEG qualidade {q}: mediana {mediana:.2} ms, pior {pior:.2} ms, {} bytes",
            jpeg.len()
        );
        if let Some(p) = pasta {
            std::fs::write(format!("{p}/q{q}.jpg"), &jpeg)?;
        }
    }
    Ok(())
}
