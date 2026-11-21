//! Gravar e reproduzir: a sessão gravada, passada pelo mesmo caminho, dá o mesmo mundo que deu ao vivo, registro a registro.

mod comum;

use comum::{amostra, caixa_em, parede_em};
use rms1_mundo::gravacao::{self, Registro};
use rms1_mundo::{Agora, Ajustes, Mundo, Percepcao};

const K: f64 = 0.1;

/// Uma sessão curta, com a torre girando, o robô andando, uma falha dita pela estação e uma grade de outro leitor: devolve o
/// mundo depois de cada grade e de cada falha, como a reprodução devolve.
fn sessao(m: &mut Mundo) -> Vec<(f64, Agora)> {
    let olho = comum::olho_do_simulador();
    let cena = [caixa_em(1.0, 0.0), caixa_em(0.3, 1.2), parede_em(4.0)];
    let mut vistos = Vec::new();
    let mut t = 5.0;
    for (giro, inclinacao, x) in [
        (0.0, 0.0, 0.0),
        (20.0, -5.0, 0.0),
        (60.0, -10.0, 0.05),
        (90.0, 0.0, 0.1),
        (30.0, 0.0, 0.2),
        (0.0, 0.0, 0.3),
    ] {
        // Três amostras por grade, e a grade casada com a do meio.
        let a = amostra(t, giro, inclinacao, [x, 0.0, 0.0]);
        m.amostra(amostra(t - 0.02, giro - 1.0, inclinacao, [x, 0.0, 0.0]));
        m.amostra(a);
        m.amostra(amostra(t + 0.02, giro + 1.0, inclinacao, [x, 0.0, 0.0]));
        m.grade(&comum::ver(&olho, &a, &cena, K), "scdepthv3", t)
            .unwrap();
        vistos.push((t, m.agora(t)));
        t += 0.07;
    }
    m.falha(t, "o quadro em h264 não abriu");
    vistos.push((t, m.agora(t)));
    let a = amostra(t + 0.07, 0.0, 0.0, [0.3, 0.0, 0.0]);
    m.amostra(a);
    m.grade(&comum::ver(&olho, &a, &cena, K), "fast_depth", t + 0.07)
        .unwrap();
    vistos.push((t + 0.07, m.agora(t + 0.07)));
    vistos
}

#[test]
fn a_sessao_reproduzida_da_o_mesmo_mundo_que_deu_ao_vivo() {
    let pasta = tempfile::tempdir().unwrap();
    let arquivo = pasta.path().join("sessao.mundo");
    let ajustes = Ajustes {
        gravar: arquivo.to_str().unwrap().to_owned(),
        ..comum::ajustes()
    };
    let olho = comum::olho_do_simulador();
    let mut m = Mundo::novo(Some(olho), comum::corpo(), ajustes.clone()).unwrap();
    assert!(m.gravando());
    let ao_vivo = sessao(&mut m);
    drop(m);
    assert!(
        ao_vivo.iter().any(|(_, a)| a.percepcao == Percepcao::Ok
            && a.obstaculo.is_some_and(|o| o.ahead_m.is_finite()))
    );
    assert!(
        ao_vivo
            .iter()
            .any(|(_, a)| a.percepcao == Percepcao::Falhou)
            && ao_vivo
                .iter()
                .any(|(_, a)| a.percepcao == Percepcao::SemMetros)
    );

    let lida = gravacao::ler(&arquivo).unwrap();
    assert_eq!(
        (lida.olho, lida.corpo),
        (Some(olho), comum::corpo()),
        "a gravação leva a câmera e o corpo"
    );
    let (amostras, grades, falhas) =
        lida.registros
            .iter()
            .fold((0, 0, 0), |(a, g, f), r| match r {
                Registro::Amostra(_) => (a + 1, g, f),
                Registro::Grade { .. } => (a, g + 1, f),
                Registro::Falha { .. } => (a, g, f + 1),
            });
    assert_eq!((amostras, grades, falhas), (19, 7, 1));
    // A reprodução não grava por cima do que lê, mesmo com os ajustes de quem gravou.
    let antes = std::fs::metadata(&arquivo).unwrap().len();
    let reproduzida = gravacao::reproduzir(&lida, None, &ajustes).unwrap();
    assert_eq!(std::fs::metadata(&arquivo).unwrap().len(), antes);
    assert_eq!(reproduzida.len(), ao_vivo.len());
    for (k, (vivo, de_novo)) in ao_vivo.iter().zip(&reproduzida).enumerate() {
        assert_eq!(
            vivo, de_novo,
            "o mundo do registro {k} não é o mesmo ao vivo e reproduzido"
        );
    }

    // Com outra câmera na reprodução, o mundo muda: é como a calibração experimenta uma lente sem voltar ao robô.
    let outra = gravacao::reproduzir(&lida, Some(comum::olho_de_teste()), &ajustes).unwrap();
    assert_ne!(outra, reproduzida);

    // A sessão interrompida no meio de um registro é lida até o último inteiro.
    let bytes = std::fs::read(&arquivo).unwrap();
    let cortado = pasta.path().join("cortado.mundo");
    std::fs::write(&cortado, &bytes[..bytes.len() - 1000]).unwrap();
    let cortada = gravacao::ler(&cortado).unwrap();
    assert_eq!(
        cortada.registros[..],
        lida.registros[..lida.registros.len() - 1]
    );
    // O que não é uma gravação é recusado pelo motivo.
    let outro = pasta.path().join("outro.bin");
    std::fs::write(&outro, b"qualquer coisa").unwrap();
    assert!(
        gravacao::ler(&outro)
            .unwrap_err()
            .contains("não é uma gravação")
    );
    assert!(
        gravacao::ler(&pasta.path().join("nao-existe"))
            .unwrap_err()
            .contains("não consegui ler")
    );
}

/// A gravação para sozinha no limite, e o mundo segue: uma sessão esquecida não enche o disco do robô.
#[test]
fn a_gravacao_para_no_limite_e_o_mundo_segue() {
    let pasta = tempfile::tempdir().unwrap();
    let arquivo = pasta.path().join("curta.mundo");
    let ajustes = Ajustes {
        gravar: arquivo.to_str().unwrap().to_owned(),
        gravar_limite_mb: 1,
        ..comum::ajustes()
    };
    let olho = comum::olho_do_simulador();
    let mut m = Mundo::novo(Some(olho), comum::corpo(), ajustes).unwrap();
    let cena = [caixa_em(1.0, 0.0), parede_em(4.0)];
    // Cada grade tem 115.216 bytes: a décima não cabe em 1 MiB.
    for k in 0..12 {
        let a = amostra(1.0 + 0.07 * f64::from(k), 0.0, 0.0, [0.0; 3]);
        m.amostra(a);
        let t = m
            .grade(&comum::ver(&olho, &a, &cena, K), "scdepthv3", a.t)
            .unwrap();
        assert_eq!(
            t.percepcao,
            Percepcao::Ok,
            "o mundo segue com a gravação parada"
        );
        assert_eq!(m.gravando(), k < 9, "na grade {k}");
    }
    drop(m);
    let tamanho = std::fs::metadata(&arquivo).unwrap().len();
    assert!(tamanho <= 1024 * 1024, "{tamanho}");
    let lida = gravacao::ler(&arquivo).unwrap();
    assert_eq!(
        lida.registros
            .iter()
            .filter(|r| matches!(r, Registro::Grade { .. }))
            .count(),
        9
    );

    // O arquivo que não abre é erro na partida, e não uma gravação que não acontece em silêncio.
    let sem_pasta = Ajustes {
        gravar: pasta
            .path()
            .join("nao-existe")
            .join("x.mundo")
            .to_str()
            .unwrap()
            .to_owned(),
        ..comum::ajustes()
    };
    assert!(Mundo::novo(Some(olho), comum::corpo(), sem_pasta).is_err());
}

/// A reprodução em passos leva, de cada grade, o que a calibração mede: a escala que a levou aos metros, o exame da faixa de
/// baixo, e as células que aquele quadro marcou como ocupadas.
#[test]
fn a_reproducao_em_passos_leva_a_escala_o_exame_e_as_ocupadas_de_cada_grade() {
    let pasta = tempfile::tempdir().unwrap();
    let arquivo = pasta.path().join("sessao.mundo");
    let ajustes = Ajustes {
        gravar: arquivo.to_str().unwrap().to_owned(),
        ..comum::ajustes()
    };
    let mut m = Mundo::novo(
        Some(comum::olho_do_simulador()),
        comum::corpo(),
        ajustes.clone(),
    )
    .unwrap();
    let ao_vivo = sessao(&mut m);
    drop(m);
    let lida = gravacao::ler(&arquivo).unwrap();
    let passos = gravacao::reproduzir_em_passos(&lida, None, &ajustes).unwrap();
    // Os mesmos mundos da reprodução de sempre, na mesma ordem.
    let de_sempre = gravacao::reproduzir(&lida, None, &ajustes).unwrap();
    assert_eq!(
        passos
            .iter()
            .map(|p| (p.t, p.agora.clone()))
            .collect::<Vec<_>>(),
        de_sempre
    );
    assert_eq!(passos.len(), ao_vivo.len());

    // A primeira grade: a torre nivelada, de frente para a caixa a 1 m. A escala é a do quadro e é a do modelo; o exame a
    // aceitou; e as ocupadas deste quadro estão na face da caixa.
    let primeira = &passos[0];
    let tratada = primeira
        .tratada
        .expect("o passo de uma grade leva o que ela deu");
    let valor = tratada
        .valor
        .expect("a escala que levou a grade aos metros");
    assert!(
        (valor / K - 1.0).abs() < 0.02,
        "a escala saiu {valor}, e a do modelo é {K}"
    );
    assert_eq!(tratada.exame.map(|e| e.recusa), Some(None), "{tratada:?}");
    assert_eq!(tratada.exame.and_then(|e| e.aceito()), tratada.do_quadro);
    assert!(
        !primeira.ocupadas.is_empty(),
        "o quadro que vê a caixa marca ocupadas"
    );
    let na_face = primeira
        .ocupadas
        .iter()
        .filter(|c| (c[0] - 1.0).abs() < 0.08 && c[1].abs() < 0.2)
        .count();
    assert!(
        na_face >= 3,
        "das {} ocupadas do primeiro quadro, {na_face} estão na face da caixa a 1 m: {:?}",
        primeira.ocupadas.len(),
        primeira.ocupadas
    );
    // E nenhuma no chão vazio: toda ocupada do quadro está numa das duas caixas ou na parede, a menos de duas células.
    let folga = 0.08;
    let em_algo = |c: &[f64; 2]| {
        let na_caixa = |x: f64, y: f64| {
            c[0] > x - folga && c[0] < x + 0.3 + folga && (c[1] - y).abs() < 0.15 + folga
        };
        na_caixa(1.0, 0.0) || na_caixa(0.3, 1.2) || c[0] > 4.0 - folga
    };
    let no_chao: Vec<_> = primeira.ocupadas.iter().filter(|c| !em_algo(c)).collect();
    assert!(
        no_chao.is_empty(),
        "o primeiro quadro deu por ocupadas células do chão vazio: {no_chao:?}"
    );

    // O passo da falha dita pela estação não é de grade nenhuma; o da grade de outro leitor é, e não tem exame nem escala.
    let da_falha = &passos[passos.len() - 2];
    assert_eq!(da_falha.agora.percepcao, Percepcao::Falhou);
    assert!(da_falha.tratada.is_none() && da_falha.ocupadas.is_empty());
    let de_outro_leitor = passos
        .last()
        .unwrap()
        .tratada
        .expect("é o passo de uma grade");
    assert_eq!(
        (
            de_outro_leitor.percepcao,
            de_outro_leitor.exame,
            de_outro_leitor.valor
        ),
        (Percepcao::SemMetros, None, None)
    );
}
