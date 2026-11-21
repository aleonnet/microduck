//! A escala pelo chão: a grade do `scdepthv3` de uma cena com o chão e uma caixa vira metros, com a torre em qualquer
//! inclinação e seja qual for a escala do modelo; e o quadro sem chão bastante não é dado por medido.

mod comum;

use comum::{amostra, br, caixa_em, entrega, parede_em, profundidades};
use rms1_mundo::escala::{self, DoQuadro, Recusa};
use rms1_mundo::grade::{self, Conversor};
use rms1_mundo::{Amostra, Escala, Olho, Percepcao, Raios, Vista};

/// A escala do quadro e, com ela, a profundidade em metros do ponto da grade em que um ponto do corpo aparece.
fn medir(olho: &Olho, a: &Amostra, carga: &[u8], alvo: [f64; 3]) -> (Option<DoQuadro>, f64, f64) {
    let g = grade::ler(carga).unwrap();
    let mut conversor = Conversor::default();
    conversor.preparar(&g);
    let vista = Vista::nova(olho, a);
    let raios = Raios::da_grade(olho, g.largura, g.altura);
    let achada = escala::do_quadro(&g, &conversor, &vista, &raios, &comum::ajustes());
    let [xn, yn, z] = vista.da_lente(alvo).expect("o alvo está à frente da lente");
    let [u, v] = olho.no_quadro(xn, yn);
    let (i, j) = (
        (u * g.largura as f64 / olho.lente.largura) as usize,
        (v * g.altura as f64 / olho.lente.altura) as usize,
    );
    assert!(
        i < g.largura && j < g.altura,
        "o alvo {alvo:?} cai fora do quadro: ({i}, {j})"
    );
    let medido = achada.map_or(f64::NAN, |e| {
        e.valor / f64::from(conversor.inverso(&g, j * g.largura + i))
    });
    (achada, medido, z)
}

/// O chão, uma parede ao fundo e uma caixa com a face a `distancia` do centro do corpo, vista com a torre de -20° a +5°, em
/// três escalas do modelo, pelas duas câmeras: a escala e a distância da caixa saem com erro de até `erro`.
fn a_caixa_a(distancia: f64, ruido: Option<f64>, erro: f64) {
    let caixas = [caixa_em(distancia, 0.0), parede_em(4.0)];
    for (nome, olho) in [
        ("simulador", comum::olho_do_simulador()),
        ("teste", comum::olho_de_teste()),
    ] {
        for inclinacao in [-20.0, -10.0, 0.0, 5.0] {
            let a = amostra(0.0, 0.0, inclinacao, [0.0; 3]);
            let prof = profundidades(&olho, &a, &caixas);
            let mut distancias = Vec::new();
            for k in [0.05, 0.1, 0.2] {
                let carga = entrega(&prof, k, ruido.map(|f| (7, f)));
                // O meio da face de perto da caixa, a 12 cm do chão.
                let (achada, medido, de_verdade) =
                    medir(&olho, &a, &carga, [distancia + 0.001, 0.0, 0.12]);
                let onde = format!(
                    "com a caixa a {} m, a torre a {inclinacao}° e a câmera do {nome}",
                    br(distancia)
                );
                let e = achada.unwrap_or_else(|| panic!("{onde}, o quadro não deu escala"));
                assert!(
                    (e.valor / k - 1.0).abs() <= erro,
                    "{onde}, a escala saiu {} e a do modelo é {k} (chão {:.2}, desvio {:.3})",
                    e.valor,
                    e.chao,
                    e.desvio
                );
                assert!(
                    (medido / de_verdade - 1.0).abs() <= erro,
                    "{onde}, a caixa a {} m saiu a {} m, com a escala do modelo em {k}",
                    br(de_verdade),
                    br(medido)
                );
                distancias.push(medido);
            }
            // A mesma distância nas três escalas: a escala do modelo não muda os metros.
            let (menor, maior) = (
                distancias.iter().copied().fold(f64::INFINITY, f64::min),
                distancias.iter().copied().fold(0.0, f64::max),
            );
            assert!(
                maior / menor - 1.0 <= erro,
                "com a caixa a {} m, as três escalas do modelo deram distâncias diferentes: {distancias:?}",
                br(distancia)
            );
        }
    }
}

#[test]
fn a_caixa_a_0_60_m_sai_na_distancia_dela() {
    a_caixa_a(0.6, None, 0.02);
}

#[test]
fn a_caixa_a_1_00_m_sai_na_distancia_dela() {
    a_caixa_a(1.0, None, 0.02);
}

#[test]
fn a_caixa_a_2_00_m_sai_na_distancia_dela() {
    a_caixa_a(2.0, None, 0.02);
}

/// Com 5% de ruído em cada ponto da grade, a escala e a distância ficam dentro de 5%.
#[test]
fn com_ruido_a_caixa_a_1_00_m_fica_dentro_de_cinco_por_cento() {
    a_caixa_a(1.0, Some(0.05), 0.05);
}

/// Uma caixa larga e muito perto, que toma perto de 60% da faixa de baixo: o chão que sobra basta, e a escala é a dele.
#[test]
fn a_caixa_que_toma_60_por_cento_da_faixa_de_baixo_nao_muda_a_escala() {
    for (nome, olho) in [
        ("simulador", comum::olho_do_simulador()),
        ("teste", comum::olho_de_teste()),
    ] {
        let a = amostra(0.0, 0.0, 0.0, [0.0; 3]);
        let de = (comum::ajustes().semente_de * comum::ALTURA as f64).ceil() as usize;
        let so_chao = profundidades(&olho, &a, &[]);
        // A caixa vai do lado direito até onde ela toma a fração da faixa mais perto de 60%.
        let tomada = |ate: f64| {
            let caixa = comum::Caixa {
                x: [0.25, 0.5],
                y: [-5.0, ate],
                altura: 0.25,
            };
            let prof = profundidades(&olho, &a, &[caixa]);
            let faixa = de * comum::LARGURA..comum::ALTURA * comum::LARGURA;
            let mais_perto = faixa
                .clone()
                .filter(|&k| prof[k] < so_chao[k] - 1e-9)
                .count();
            (mais_perto as f64 / faixa.len() as f64, prof)
        };
        let (fracao, prof) = (-50..=50)
            .map(|c| tomada(f64::from(c) / 100.0))
            .min_by(|a, b| (a.0 - 0.6).abs().total_cmp(&(b.0 - 0.6).abs()))
            .unwrap();
        assert!(
            (0.55..=0.65).contains(&fracao),
            "a cena do {nome} não ficou com 60% da faixa tomada: {fracao}"
        );
        for k in [0.05, 0.2] {
            let (achada, _, _) = medir(&olho, &a, &entrega(&prof, k, None), [1.0, 1.0, 0.0]);
            let e = achada.unwrap_or_else(|| {
                panic!("com {fracao:.2} da faixa tomada, a câmera do {nome} não deu escala")
            });
            assert!(
                (e.valor / k - 1.0).abs() <= 0.02 && e.chao >= 0.3,
                "com a caixa tomando {fracao:.2} da faixa de baixo, a escala saiu {} e a do modelo é {k} (chão {:.2})",
                e.valor,
                e.chao
            );
        }
    }
}

/// A faixa de baixo sem chão bastante (uma parede a um palmo): a escala do quadro não vale; vale a lembrada, dita como
/// lembrada, enquanto o prazo dura; passado ele, ou sem nenhuma, não há metros.
#[test]
fn sem_chao_bastante_vale_a_escala_lembrada_e_so_dentro_do_prazo() {
    let olho = comum::olho_do_simulador();
    let a = amostra(10.0, 0.0, 0.0, [0.0; 3]);
    let com_chao = comum::ver(&olho, &a, &[caixa_em(1.0, 0.0), parede_em(4.0)], 0.1);
    // Uma parede a 20 cm do centro do corpo, 12 cm à frente da lente: toma o quadro inteiro.
    let sem_chao = comum::ver(&olho, &a, &[parede_em(0.2)], 0.1);
    let (achada, _, _) = medir(&olho, &a, &sem_chao, [1.0, 0.0, 0.1]);
    assert_eq!(
        achada, None,
        "a parede que enche a faixa de baixo não é chão, e não dá escala"
    );

    // Sem nenhuma escala de antes: não há metros.
    let mut m = comum::mundo(olho);
    m.amostra(a);
    let t = m.grade(&sem_chao, "scdepthv3", 10.0).unwrap();
    assert_eq!((t.percepcao, t.escala), (Percepcao::SemChao, None));
    assert_eq!(m.agora(10.0).percepcao.nome(), "no_floor");

    // Com uma escala boa de meio segundo antes: vale a lembrada, e o mundo diz que é lembrada.
    let mut m = comum::mundo(olho);
    m.amostra(a);
    let t = m.grade(&com_chao, "scdepthv3", 10.0).unwrap();
    assert_eq!(
        (t.percepcao, t.escala),
        (Percepcao::Ok, Some(Escala::Medida))
    );
    m.amostra(amostra(10.5, 0.0, 0.0, [0.0; 3]));
    let t = m.grade(&sem_chao, "scdepthv3", 10.5).unwrap();
    assert_eq!(
        (t.percepcao, t.escala),
        (Percepcao::Ok, Some(Escala::Lembrada)),
        "a escala lembrada"
    );
    assert_eq!(m.agora(10.5).linha()["perception"]["scale"], "remembered");
    // A parede, medida com a escala lembrada, sai onde está: a 20 cm do centro, 4 cm à frente do chassi.
    let o = m.agora(10.5).obstaculo.expect("a parede está no corredor");
    assert!(
        (o.ahead_m - 0.04).abs() <= 0.05,
        "a parede a 4 cm da frente do chassi saiu a {} m",
        o.ahead_m
    );

    // Passado o prazo da escala lembrada, o mesmo quadro não dá metros.
    let depois = 10.0 + comum::ajustes().escala_vale_ms as f64 / 1000.0 + 0.5;
    m.amostra(amostra(depois, 0.0, 0.0, [0.0; 3]));
    let t = m.grade(&sem_chao, "scdepthv3", depois).unwrap();
    assert_eq!(
        (t.percepcao, t.escala),
        (Percepcao::SemChao, None),
        "passado o prazo, a escala lembrada não vale"
    );
}

/// A torre olhando para cima: o chão sai da faixa de baixo pela geometria, e o quadro não é dado por medido.
#[test]
fn com_a_torre_para_cima_o_chao_sai_da_faixa_e_o_quadro_nao_e_medido() {
    let olho = comum::olho_de_teste();
    let a = amostra(0.0, 0.0, 30.0, [0.0; 3]);
    let carga = comum::ver(&olho, &a, &[parede_em(4.0)], 0.1);
    let (achada, _, _) = medir(&olho, &a, &carga, [3.9, 0.0, 2.0]);
    assert_eq!(
        achada, None,
        "a 30° para cima a faixa de baixo não mostra o chão ao alcance"
    );
}

/// A grade casada com a inclinação errada: o que a faixa mostra não é o chão que a conta espera. O quadro só é dado por medido
/// com o erro de escala dentro da tolerância (mais dois pontos, do que a conferência não vê), e com 10° ou mais de erro ele
/// nunca é dado por medido. O erro pequeno de inclinação passa, e o teste diz quanto de escala cada um custa.
#[test]
fn com_a_inclinacao_errada_o_quadro_nao_e_dado_por_medido() {
    let tolerancia = comum::ajustes().tolerancia;
    for (nome, olho) in [
        ("simulador", comum::olho_do_simulador()),
        ("teste", comum::olho_de_teste()),
    ] {
        for de_verdade in [-20.0, -10.0, 0.0] {
            let prof = profundidades(
                &olho,
                &amostra(0.0, 0.0, de_verdade, [0.0; 3]),
                &[parede_em(6.0)],
            );
            let carga = entrega(&prof, 0.1, None);
            for erro in [
                -20.0, -15.0, -10.0, -5.0, -2.0, 2.0, 5.0, 10.0, 15.0, 20.0f64,
            ] {
                let dita = amostra(0.0, 0.0, de_verdade + erro, [0.0; 3]);
                let (achada, _, _) = medir(&olho, &dita, &carga, [5.9, 0.0, 2.5]);
                println!(
                    "{nome}: torre a {de_verdade}°, dita a {}°: {}",
                    de_verdade + erro,
                    achada.map_or("sem escala".to_owned(), |e| format!(
                        "escala com erro de {:+.1}%, desvio {:.3}",
                        (e.valor / 0.1 - 1.0) * 100.0,
                        e.desvio
                    ))
                );
                if let Some(e) = achada {
                    assert!(
                        erro.abs() < 10.0 && (e.valor / 0.1 - 1.0).abs() <= tolerancia + 0.02,
                        "com a câmera do {nome}, a torre a {de_verdade}° e a grade casada com {}°, o quadro foi dado por medido, com a escala em {} no lugar de 0,1",
                        de_verdade + erro,
                        e.valor
                    );
                }
            }
        }
    }
}

/// O exame da faixa de baixo diz o que a conta viu, valha a escala ou não: é o que a calibração lê para saber por que um
/// quadro não foi dado por medido.
#[test]
fn o_exame_diz_por_que_a_escala_do_quadro_nao_valeu() {
    let olho = comum::olho_de_teste();
    let examinar = |dita: &Amostra, carga: &[u8]| {
        let g = grade::ler(carga).unwrap();
        let mut conversor = Conversor::default();
        conversor.preparar(&g);
        let vista = Vista::nova(&olho, dita);
        let raios = Raios::da_grade(&olho, g.largura, g.altura);
        let ajustes = comum::ajustes();
        let exame = escala::examinar(&g, &conversor, &vista, &raios, &ajustes);
        assert_eq!(
            exame.aceito(),
            escala::do_quadro(&g, &conversor, &vista, &raios, &ajustes),
            "o exame aceito é a escala do quadro"
        );
        // A faixa de baixo é o quinto de baixo da grade de 320 por 180.
        assert_eq!(exame.total, 36 * 320);
        assert!(
            exame.ficaram <= exame.pontos && exame.pontos <= exame.total,
            "{exame:?}"
        );
        exame
    };
    let nivelada = amostra(0.0, 0.0, 0.0, [0.0; 3]);
    let chao = comum::ver(&olho, &nivelada, &[parede_em(4.0)], 0.1);

    // O chão limpo: a escala vale, e as duas contas dela dão a do modelo.
    let e = examinar(&nivelada, &chao);
    assert_eq!(e.recusa, None, "{e:?}");
    for (nome, conta) in [("pela razão", e.pela_razao), ("pela reta", e.pela_reta)] {
        let valor = conta.unwrap_or_else(|| panic!("o chão limpo não deu a escala {nome}: {e:?}"));
        assert!(
            (valor / 0.1 - 1.0).abs() < 0.02,
            "no chão limpo a escala {nome} saiu {valor}, e a do modelo é 0,1"
        );
    }
    assert!(e.desvio.unwrap() < 0.02 && e.ficaram > e.total / 2, "{e:?}");

    // A torre a 30° para cima: a faixa de baixo não tem ponto que possa ser chão, e é isso que o exame diz.
    let para_cima = amostra(0.0, 0.0, 30.0, [0.0; 3]);
    let e = examinar(
        &para_cima,
        &comum::ver(&olho, &para_cima, &[parede_em(4.0)], 0.1),
    );
    assert_eq!((e.recusa, e.pontos), (Some(Recusa::PoucoChao), 0), "{e:?}");

    // A grade do chão limpo casada com a torre 15° acima do que estava: recusada, e não por falta de pontos (a faixa segue
    // abaixo do horizonte): o que ela mostra não é o chão que a conta espera.
    // As duas contas da escala se separam (a pela razão dá mais que o dobro da pela reta), e o motivo dito é o desvio.
    let e = examinar(&amostra(0.0, 0.0, 15.0, [0.0; 3]), &chao);
    assert!(
        e.recusa == Some(Recusa::Desvio) && e.desvio.unwrap() > 1.0 && e.pontos > e.total / 2,
        "com a torre dita 15° acima, o exame tem de recusar pelo desvio entre as duas contas: {e:?}"
    );

    // Uma parede a 45 cm enchendo a faixa de baixo: o inverso da profundidade não cresce com a linha, a escala pela reta sai
    // perto de zero, e o motivo dito é o desvio, e não a falta de pontos.
    let e = examinar(
        &nivelada,
        &comum::ver(&olho, &nivelada, &[parede_em(0.45)], 0.1),
    );
    assert!(
        e.recusa == Some(Recusa::Desvio)
            && e.pela_reta.unwrap() < 0.1 * e.pela_razao.unwrap()
            && e.pontos == e.total,
        "com a parede enchendo a faixa, o exame tem de recusar pelo desvio, com a reta deitada: {e:?}"
    );
}
