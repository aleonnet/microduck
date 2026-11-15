//! `rms1-mediad-prova`: as provas do `rms1-mediad` que rodam fora dos testes, contra arquivos e contra a estação de verdade.
//!
//!   rms1-mediad-prova reducao <quadro.nv12> <largura> <altura> <saída.rgb>
//!       reduz um quadro NV12 de 1280 por 720, como o serviço faz antes de mandar à estação, e grava o RGB.

use std::process::ExitCode;

use rms1_mediad::reducao;

/// O quadro da câmera do robô.
const LARGURA: usize = 1280;
const ALTURA: usize = 720;
/// O que o driver da câmera anexa a cada quadro.
const SOBRA_DO_DRIVER: usize = 32;

fn reduzir(args: &[String]) -> Result<(), String> {
    let [quadro, largura, altura, saida] = args else {
        return Err(
            "uso: rms1-mediad-prova reducao <quadro.nv12> <largura> <altura> <saída.rgb>"
                .to_owned(),
        );
    };
    let numero = |s: &String| {
        s.parse::<usize>()
            .map_err(|_| format!("não é um número: {s}"))
    };
    let (ls, as_) = (numero(largura)?, numero(altura)?);
    let mut nv12 = std::fs::read(quadro).map_err(|e| format!("{quadro}: {e}"))?;
    // O quadro como o driver da câmera do robô o entrega traz 32 bytes a mais no fim, que não são imagem
    // (`docs/s1/project/s1-bringup.md:953-958`): o serviço os descarta, e esta prova também.
    if nv12.len() == LARGURA * ALTURA * 3 / 2 + SOBRA_DO_DRIVER {
        nv12.truncate(LARGURA * ALTURA * 3 / 2);
    }
    let rgb = reducao::nv12_para_rgb(&nv12, LARGURA, ALTURA, ls, as_).ok_or_else(|| {
        format!(
            "{quadro} tem {} bytes, e um NV12 de {LARGURA} por {ALTURA} tem {}",
            nv12.len(),
            LARGURA * ALTURA * 3 / 2
        )
    })?;
    std::fs::write(saida, &rgb).map_err(|e| format!("{saida}: {e}"))?;
    println!("{saida}: {} bytes", rgb.len());
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let resultado = match args.split_first() {
        Some((comando, resto)) if comando == "reducao" => reduzir(resto),
        _ => Err(
            "uso: rms1-mediad-prova reducao <quadro.nv12> <largura> <altura> <saída.rgb>"
                .to_owned(),
        ),
    };
    match resultado {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rms1-mediad-prova: {e}");
            ExitCode::FAILURE
        }
    }
}
