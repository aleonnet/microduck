//! A ocupação: grades seguidas, com a torre girando, o chassi girando e o robô andando, marcam livre, ocupado e não visto nas
//! células certas, e a memória se comporta como a do cérebro do pato.

mod comum;

use comum::{amostra, caixa_em, entrega, parede_em, profundidades};
use rms1_mundo::{Amostra, Estado, Mundo, Olho};

const K: f64 = 0.1;

/// Mostra ao mundo o que a câmera vê da cena, posta em `a`.
fn olhar(m: &mut Mundo, olho: &Olho, a: Amostra, caixas: &[comum::Caixa]) {
    m.amostra(a);
    let t = m
        .grade(&comum::ver(olho, &a, caixas, K), "scdepthv3", a.t)
        .unwrap();
    assert_eq!(
        t.percepcao.nome(),
        "ok",
        "a cena tem chão, e a grade tem de virar metros: {t:?}"
    );
}

/// O estado do mapa na face de perto de uma caixa posta em `x`, no meio do caminho. As caixas dos testes têm a face na divisa
/// de duas células (1 m são 25 células de 4 cm), e os pontos medidos nela caem de um lado e do outro: a face está ocupada se
/// uma das duas células está.
fn na_face(m: &Mundo, x: f64, t: f64) -> Estado {
    let (antes, depois) = (
        m.estado_em([x - 0.01, 0.0], t),
        m.estado_em([x + 0.01, 0.0], t),
    );
    if antes == Estado::Ocupada {
        antes
    } else {
        depois
    }
}

#[test]
fn livre_ocupado_e_nao_visto_caem_nas_celulas_certas() {
    for olho in [comum::olho_do_simulador(), comum::olho_de_teste()] {
        let mut m = comum::mundo(olho);
        let cena = [caixa_em(1.0, 0.0), parede_em(4.0)];
        olhar(&mut m, &olho, amostra(1.0, 0.0, 0.0, [0.0; 3]), &cena);
        assert_eq!(
            na_face(&m, 1.0, 1.0),
            Estado::Ocupada,
            "o caminho barrado pela caixa tem de sair ocupado"
        );
        assert_eq!(
            m.estado_em([0.7, 0.0], 1.0),
            Estado::Livre,
            "o chão entre o robô e a caixa foi visto livre"
        );
        assert_eq!(
            m.estado_em([1.5, 0.3], 1.0),
            Estado::Livre,
            "o chão ao lado da caixa foi visto livre"
        );
        // Atrás da caixa, que é mais alta que a lente, nada foi visto; nem atrás do robô; nem debaixo da vista da câmera.
        assert_eq!(
            m.estado_em([1.6, 0.0], 1.0),
            Estado::NaoVista,
            "atrás da caixa nada foi visto"
        );
        assert_eq!(
            m.estado_em([-1.0, 0.0], 1.0),
            Estado::NaoVista,
            "atrás do robô nada foi visto"
        );
        assert_eq!(
            m.estado_em([0.12, 0.0], 1.0),
            Estado::NaoVista,
            "o chão debaixo da vista da câmera não foi visto"
        );
        // A parede está a 4 m, além do alcance: não entra no mapa.
        assert_eq!(na_face(&m, 4.0, 1.0), Estado::NaoVista);
        // O ocupado é a face da caixa, e não o chão à frente dela.
        assert_eq!(
            m.estado_em([0.9, 0.0], 1.0),
            Estado::Livre,
            "a 10 cm da caixa o chão é livre"
        );

        // Uma caixa mais baixa que a lente fica inteira abaixo do horizonte: só a conta do chão a separa do chão.
        let mut m = comum::mundo(olho);
        let baixa = comum::Caixa {
            x: [0.8, 1.1],
            y: [-0.15, 0.15],
            altura: 0.15,
        };
        olhar(
            &mut m,
            &olho,
            amostra(1.0, 0.0, 0.0, [0.0; 3]),
            &[baixa, parede_em(4.0)],
        );
        assert_eq!(
            na_face(&m, 0.8, 1.0),
            Estado::Ocupada,
            "o caminho barrado pela caixa baixa tem de sair ocupado"
        );
        assert_ne!(
            m.estado_em([0.95, 0.0], 1.0),
            Estado::Livre,
            "o lugar da caixa baixa não é chão livre"
        );
        assert_eq!(
            m.estado_em([0.6, 0.0], 1.0),
            Estado::Livre,
            "o chão antes da caixa baixa foi visto livre"
        );
    }
}

#[test]
fn o_ocupado_fica_no_lugar_com_a_torre_virada_o_chassi_girado_e_o_robo_andando() {
    let olho = comum::olho_do_simulador();
    let mut m = comum::mundo(olho);
    let cena = [caixa_em(1.0, 0.0), caixa_em(0.3, 1.2), parede_em(4.0)];
    olhar(&mut m, &olho, amostra(1.0, 0.0, 0.0, [0.0; 3]), &cena);
    assert_eq!(
        na_face(&m, 1.0, 1.0),
        Estado::Ocupada,
        "o caminho barrado pela caixa"
    );
    // A torre vira 90° para a esquerda: a caixa da frente sai da vista, e segue lembrada onde estava.
    olhar(&mut m, &olho, amostra(1.2, 90.0, 0.0, [0.0; 3]), &cena);
    assert_eq!(
        na_face(&m, 1.0, 1.2),
        Estado::Ocupada,
        "com a torre virada, o ocupado tem de seguir lembrado"
    );
    // E a caixa da esquerda, que a torre agora vê, entra: a face dela está em y = 1,05.
    assert_eq!(
        m.estado_em([0.45, 1.06], 1.2),
        Estado::Ocupada,
        "a caixa da esquerda, vista com a torre virada"
    );
    // O chassi gira 90° à esquerda com a torre a -90°: a câmera olha para onde olhava no começo, e a caixa é a mesma célula.
    olhar(
        &mut m,
        &olho,
        amostra(1.4, -90.0, 0.0, [0.0, 0.0, 90.0]),
        &cena,
    );
    assert_eq!(
        na_face(&m, 1.0, 1.4),
        Estado::Ocupada,
        "com o chassi girado, a caixa segue na mesma célula"
    );
    // O robô anda 30 cm para a frente: a caixa não anda com ele.
    olhar(
        &mut m,
        &olho,
        amostra(1.6, 0.0, 0.0, [0.3, 0.0, 0.0]),
        &cena,
    );
    assert_eq!(
        na_face(&m, 1.0, 1.6),
        Estado::Ocupada,
        "com o robô andando, a caixa fica onde está"
    );
    assert_eq!(
        m.estado_em([1.31, 0.0], 1.6),
        Estado::NaoVista,
        "o robô andou, e a caixa não andou com ele"
    );
    // Com a torre inclinada 20° para baixo (o limite dela), o chão é visto de mais perto: com a lente do robô, nivelada, a
    // linha de baixo do quadro vê o chão a 53 cm do centro do corpo; a 20° para baixo, a 27 cm.
    let mut m = comum::mundo(olho);
    olhar(&mut m, &olho, amostra(2.0, 0.0, 0.0, [0.0; 3]), &cena);
    assert_eq!(
        m.estado_em([0.3, 0.0], 2.0),
        Estado::NaoVista,
        "com a torre nivelada, o chão a 30 cm fica abaixo do quadro"
    );
    let mut m = comum::mundo(olho);
    olhar(&mut m, &olho, amostra(2.0, 0.0, -20.0, [0.0; 3]), &cena);
    assert_eq!(
        m.estado_em([0.3, 0.0], 2.0),
        Estado::Livre,
        "com a torre para baixo, o chão a 30 cm é visto"
    );
    assert_eq!(
        na_face(&m, 1.0, 2.0),
        Estado::Ocupada,
        "o caminho barrado, com a torre para baixo"
    );
}

/// Uma coluna da grade com três pontos mais perto que o chão, e mais nada: `colunas` diz em quais.
fn com_espinho(olho: &Olho, a: &Amostra, colunas: &[usize]) -> Vec<u8> {
    let mut prof = profundidades(olho, a, &[]);
    for &i in colunas {
        for j in 120..123 {
            prof[j * comum::LARGURA + i] *= 0.6;
        }
    }
    entrega(&prof, K, None)
}

fn ocupadas(m: &Mundo, t: f64) -> usize {
    m.celulas(t)
        .iter()
        .filter(|c| c.1 == Estado::Ocupada)
        .count()
}

#[test]
fn um_ponto_ocupado_so_vale_com_vizinho() {
    let olho = comum::olho_do_simulador();
    // Uma coluna sozinha, num quadro só: ruído, e não obstáculo.
    let mut m = comum::mundo(olho);
    let a = amostra(1.0, 0.0, 0.0, [0.0; 3]);
    m.amostra(a);
    m.grade(&com_espinho(&olho, &a, &[160]), "scdepthv3", 1.0)
        .unwrap();
    assert_eq!(
        ocupadas(&m, 1.0),
        0,
        "uma coluna sozinha, sem vizinho, não põe ocupado no mapa"
    );
    // A mesma coluna no quadro seguinte: o vizinho é o do quadro anterior.
    let a = amostra(1.07, 0.0, 0.0, [0.0; 3]);
    m.amostra(a);
    m.grade(&com_espinho(&olho, &a, &[160]), "scdepthv3", 1.07)
        .unwrap();
    assert!(
        ocupadas(&m, 1.07) > 0,
        "com o vizinho do quadro anterior, o ponto vale"
    );
    // Duas colunas vizinhas no mesmo quadro: valem na hora.
    let mut m = comum::mundo(olho);
    let a = amostra(1.0, 0.0, 0.0, [0.0; 3]);
    m.amostra(a);
    m.grade(&com_espinho(&olho, &a, &[160, 161]), "scdepthv3", 1.0)
        .unwrap();
    assert!(
        ocupadas(&m, 1.0) > 0,
        "com o vizinho da coluna ao lado, o ponto vale"
    );
    // Um ponto só na coluna, sem o de cima: nem candidato é.
    let mut m = comum::mundo(olho);
    let mut prof = profundidades(&olho, &a, &[]);
    for i in 150..170 {
        prof[120 * comum::LARGURA + i] *= 0.6;
    }
    m.amostra(a);
    m.grade(&entrega(&prof, K, None), "scdepthv3", 1.0).unwrap();
    assert_eq!(
        ocupadas(&m, 1.0),
        0,
        "uma linha só, sem o segundo ponto seguido da coluna, não é obstáculo"
    );
}

#[test]
fn livre_nao_apaga_o_lembrado_e_o_que_nao_e_visto_de_novo_e_esquecido() {
    let olho = comum::olho_do_simulador();
    let ajustes = comum::ajustes();
    let mut m = comum::mundo(olho);
    let com_caixa = [caixa_em(1.0, 0.0), parede_em(4.0)];
    let sem_caixa = [parede_em(4.0)];
    olhar(&mut m, &olho, amostra(1.0, 0.0, 0.0, [0.0; 3]), &com_caixa);
    assert_eq!(
        na_face(&m, 1.0, 1.0),
        Estado::Ocupada,
        "o caminho barrado pela caixa"
    );
    // A caixa some da cena. Os primeiros quadros que veem o chão no lugar dela não a apagam.
    let mut t = 1.0;
    for quadro in 1..ajustes.confirma_livre {
        t += 0.07;
        olhar(&mut m, &olho, amostra(t, 0.0, 0.0, [0.0; 3]), &sem_caixa);
        assert_eq!(
            na_face(&m, 1.0, t),
            Estado::Ocupada,
            "um quadro livre não apaga o lembrado: caiu no quadro {quadro} de {}",
            ajustes.confirma_livre
        );
    }
    // No quadro que completa a conta, a célula volta a livre.
    t += 0.07;
    olhar(&mut m, &olho, amostra(t, 0.0, 0.0, [0.0; 3]), &sem_caixa);
    assert_eq!(
        na_face(&m, 1.0, t),
        Estado::Livre,
        "vista livre em quadros seguidos, a célula deixa de ser ocupada"
    );

    // Os quadros livres têm de ser seguidos: com a caixa de volta no meio, a conta recomeça.
    let mut m = comum::mundo(olho);
    olhar(&mut m, &olho, amostra(1.0, 0.0, 0.0, [0.0; 3]), &com_caixa);
    for (k, cena) in [
        &sem_caixa[..],
        &sem_caixa[..],
        &com_caixa[..],
        &sem_caixa[..],
        &sem_caixa[..],
    ]
    .into_iter()
    .enumerate()
    {
        let t = 1.07 + 0.07 * k as f64;
        olhar(&mut m, &olho, amostra(t, 0.0, 0.0, [0.0; 3]), cena);
        assert_eq!(
            na_face(&m, 1.0, t),
            Estado::Ocupada,
            "o lembrado caiu antes de três quadros livres seguidos, no quadro {k}"
        );
    }

    // O que a torre deixou de olhar é esquecido depois do tempo de lembrança, e só depois.
    let mut m = comum::mundo(olho);
    olhar(&mut m, &olho, amostra(1.0, 0.0, 0.0, [0.0; 3]), &com_caixa);
    let quase = 1.0 + ajustes.lembra_s - 0.1;
    assert_eq!(
        na_face(&m, 1.0, quase),
        Estado::Ocupada,
        "o ocupado segue lembrado até o fim do prazo"
    );
    assert_eq!(
        na_face(&m, 1.0, quase + 0.2),
        Estado::NaoVista,
        "passado o prazo, o lembrado é esquecido"
    );
    assert_eq!(
        m.estado_em([0.7, 0.0], quase + 0.2),
        Estado::NaoVista,
        "e o livre também"
    );
    assert!(m.celulas(quase + 0.2).is_empty());
}

/// A janela anda com o robô: o que ficou longe sai do mapa, e o lugar é reusado sem trazer célula velha.
#[test]
fn a_janela_do_mapa_anda_com_o_robo() {
    let olho = comum::olho_do_simulador();
    let mut m = comum::mundo(olho);
    olhar(
        &mut m,
        &olho,
        amostra(1.0, 0.0, 0.0, [0.0; 3]),
        &[caixa_em(1.0, 0.0)],
    );
    assert_eq!(
        na_face(&m, 1.0, 1.0),
        Estado::Ocupada,
        "o caminho barrado pela caixa"
    );
    // O robô vai para 20 m dali, com outra caixa à frente: a primeira ficou fora da janela, de 8 m.
    olhar(
        &mut m,
        &olho,
        amostra(1.5, 0.0, 0.0, [20.0, 0.0, 0.0]),
        &[caixa_em(21.0, 0.0)],
    );
    assert_eq!(
        na_face(&m, 1.0, 1.5),
        Estado::NaoVista,
        "a caixa que ficou a 20 m saiu da janela"
    );
    assert_eq!(
        na_face(&m, 21.0, 1.5),
        Estado::Ocupada,
        "a caixa nova, à frente do robô onde ele está"
    );
    assert!(
        m.celulas(1.5)
            .iter()
            .all(|(c, _, _)| (f64::from(c[0]) * 0.04 - 20.0).abs() < 4.1),
        "só as células da janela de agora"
    );
}
