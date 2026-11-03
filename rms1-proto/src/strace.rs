//! As leituras de um `strace`, como bytes, para reprocessar gravações do robô. Sem E/S: quem chama entrega o texto.
//!
//! Cobre o que as gravações do laboratório têm: `read(fd, "...", n)` com ou sem `-f` (o número do processo na frente) e
//! com ou sem `-tt` (a hora), e a leitura partida em duas linhas, `read(fd, <unfinished ...>` e depois, do mesmo
//! processo, `<... read resumed> "...", n)`. A primeira versão, dentro de `tests/gravacoes_lab.rs`, perdia a segunda
//! forma: de 2 a 4529 leituras por gravação (medido em 2026-11-03 com `grep -c 'read resumed> "'`).
//!
//! Texto cortado pelo `-s` do `strace` termina em `"...`: o que veio até o corte entra, e o resto se perdeu na gravação.
//! O [`Leitor`] acha o próximo quadro inteiro.
//!
//! **Limite:** os fluxos se separam pelo número do descritor, que é o certo para as threads de um processo, como nas
//! gravações do laboratório. Dois processos com o mesmo número de descritor no mesmo `strace` se misturariam.

use std::collections::HashMap;

use crate::{Leitor, Quadro};

/// Uma leitura de um descritor.
#[derive(Debug, Clone, PartialEq)]
pub struct Leitura {
    pub fd: i64,
    /// Segundos desde a meia-noite, quando a gravação tem `-tt`.
    pub hora_s: Option<f64>,
    pub bytes: Vec<u8>,
}

/// Desfaz o escape do `strace`: `\n`, `\t`, `\xHH`, octal de até três dígitos e os demais de C.
pub fn desfazer(s: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'\\' || i + 1 >= s.len() {
            o.push(s[i]);
            i += 1;
            continue;
        }
        let n = s[i + 1];
        let simples = match n {
            b'n' => Some(10),
            b't' => Some(9),
            b'r' => Some(13),
            b'f' => Some(12),
            b'v' => Some(11),
            b'a' => Some(7),
            b'b' => Some(8),
            b'e' => Some(27),
            b'\\' => Some(92),
            b'"' => Some(34),
            _ => None,
        };
        if let Some(v) = simples {
            o.push(v);
            i += 2;
        } else if n == b'x' && i + 4 <= s.len() {
            match std::str::from_utf8(&s[i + 2..i + 4])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                Some(v) => o.push(v),
                None => o.push(b'\\'),
            }
            i += 4;
        } else {
            let mut j = i + 1;
            let mut v = 0u32;
            while j < s.len() && j < i + 4 && (b'0'..=b'7').contains(&s[j]) {
                v = v * 8 + u32::from(s[j] - b'0');
                j += 1;
            }
            if j == i + 1 {
                // Barra seguida de algo que o `strace` não escreve: fica como veio.
                o.push(b'\\');
                i += 1;
                continue;
            }
            o.push(v as u8);
            i = j;
        }
    }
    o
}

/// O texto entre aspas que começa em `s[0] == b'"'`, sem as aspas, e com o escape ainda feito.
fn entre_aspas(s: &[u8]) -> &[u8] {
    let corpo = &s[1..];
    let mut f = 0;
    while f < corpo.len() && corpo[f] != b'"' {
        f += if corpo[f] == b'\\' { 2 } else { 1 };
    }
    &corpo[..f.min(corpo.len())]
}

/// `HH:MM:SS.ffffff` em segundos.
fn hora(token: &[u8]) -> Option<f64> {
    let t = std::str::from_utf8(token).ok()?;
    let mut partes = t.split(':');
    let (h, m, s) = (partes.next()?, partes.next()?, partes.next()?);
    if partes.next().is_some() || !s.contains('.') {
        return None;
    }
    Some(h.parse::<f64>().ok()? * 3600.0 + m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?)
}

/// O processo e a hora, do começo da linha: `[pid 123] `, `123 ` e `HH:MM:SS.ffffff `, cada um opcional.
fn cabeca(linha: &[u8]) -> (Option<Vec<u8>>, Option<f64>) {
    let mut resto = linha;
    let mut pid = None;
    if let Some(r) = resto.strip_prefix(b"[pid ")
        && let Some(f) = r.iter().position(|b| *b == b']')
    {
        pid = Some(r[..f].to_vec());
        resto = &r[f + 1..];
    }
    let mut hora_s = None;
    for _ in 0..2 {
        let r = resto.trim_ascii_start();
        let fim = r.iter().position(|b| *b == b' ').unwrap_or(r.len());
        let token = &r[..fim];
        if pid.is_none() && !token.is_empty() && token.iter().all(u8::is_ascii_digit) {
            pid = Some(token.to_vec());
            resto = &r[fim..];
        } else if let Some(h) = hora(token) {
            hora_s = Some(h);
            resto = &r[fim..];
        } else {
            break;
        }
    }
    (pid, hora_s)
}

/// Onde começa a chamada `read(`, sem pegar `pread(` nem `readv(`, e antes de qualquer aspa.
fn inicio_do_read(antes_das_aspas: &[u8]) -> Option<usize> {
    let mut desde = 0;
    while let Some(p) = antes_das_aspas[desde..]
        .windows(5)
        .position(|w| w == b"read(")
    {
        let p = desde + p;
        if p == 0 || matches!(antes_das_aspas[p - 1], b' ' | b']' | b'\t') {
            return Some(p);
        }
        desde = p + 1;
    }
    None
}

/// Todas as leituras do texto, na ordem em que terminaram.
pub fn leituras(texto: &[u8]) -> Vec<Leitura> {
    let mut pendentes: HashMap<Option<Vec<u8>>, i64> = HashMap::new();
    let mut saida = Vec::new();
    for linha in texto.split(|b| *b == b'\n') {
        let aspas = linha.iter().position(|b| *b == b'"');
        let antes = &linha[..aspas.unwrap_or(linha.len())];
        let (pid, hora_s) = cabeca(linha);
        let fd = if antes.windows(19).any(|w| w == b"<... read resumed> ") {
            match pendentes.remove(&pid) {
                Some(fd) => fd,
                None => continue,
            }
        } else if let Some(p) = inicio_do_read(antes) {
            let resto = &antes[p + 5..];
            let Some(v) = resto.iter().position(|b| *b == b',') else {
                continue;
            };
            let Some(fd) = std::str::from_utf8(&resto[..v])
                .ok()
                .and_then(|f| f.trim().parse::<i64>().ok())
            else {
                continue;
            };
            if resto.windows(11).any(|w| w == b"<unfinished") {
                pendentes.insert(pid, fd);
                continue;
            }
            fd
        } else {
            continue;
        };
        let Some(a) = aspas else {
            continue;
        };
        saida.push(Leitura {
            fd,
            hora_s,
            bytes: desfazer(entre_aspas(&linha[a..])),
        });
    }
    saida
}

/// Os quadros lidos, com o verificador conferido, um [`Leitor`] por descritor, e a hora da leitura que fechou cada um.
pub fn quadros(texto: &[u8]) -> Vec<(Option<f64>, Quadro)> {
    let mut por_fd: HashMap<i64, Leitor> = HashMap::new();
    let mut saida = Vec::new();
    for l in leituras(texto) {
        for q in por_fd.entry(l.fd).or_default().empurrar(&l.bytes) {
            saida.push((l.hora_s, q));
        }
    }
    saida
}

#[cfg(test)]
mod testes {
    use super::*;

    fn quadro_hex() -> (Vec<u8>, String) {
        let q = Quadro {
            origem: 0x03,
            destino: 0x09,
            seq: 7,
            resposta: false,
            pede: crate::PedeAck::Nada,
            cmd_set: 0x48,
            cmd_id: 0x08,
            dados: vec![0, 1, 2, 0x22, 0x5c, 10],
        }
        .montar()
        .unwrap();
        let texto: String = q.iter().map(|b| format!("\\x{b:02x}")).collect();
        (q, texto)
    }

    #[test]
    fn desfaz_os_escapes_que_o_strace_escreve() {
        assert_eq!(desfazer(br#"a\n\t\x55\0\177\\\""#), b"a\n\t\x55\0\x7f\\\"");
        assert_eq!(desfazer(br"\12a"), b"\na");
    }

    #[test]
    fn le_com_e_sem_processo_e_hora() {
        let (q, h) = quadro_hex();
        for prefixo in [
            "",
            "387   ",
            "[pid  387] ",
            "387   14:18:39.123456 ",
            "14:18:39.123456 ",
        ] {
            let t = format!("{prefixo}read(13, \"{h}\", 256) = {}\n", q.len());
            let l = leituras(t.as_bytes());
            assert_eq!(l.len(), 1, "{prefixo:?}");
            assert_eq!(l[0].fd, 13);
            assert_eq!(l[0].bytes, q);
            assert_eq!(l[0].hora_s.is_some(), prefixo.contains(':'), "{prefixo:?}");
        }
    }

    /// A forma que a primeira versão perdia: a leitura partida em duas linhas, com outro processo no meio.
    #[test]
    fn a_leitura_retomada_entra_com_o_descritor_da_linha_que_comecou() {
        let (q, h) = quadro_hex();
        let t = format!(
            "387   read(13,  <unfinished ...>\n391   read(20, \"xx\", 2) = 2\n387   <... read resumed> \"{h}\", 256) = {}\n",
            q.len()
        );
        let l = leituras(t.as_bytes());
        assert_eq!(l.len(), 2);
        assert_eq!((l[1].fd, &l[1].bytes), (13, &q));
        assert_eq!(quadros(t.as_bytes()).len(), 1);
    }

    #[test]
    fn retomada_sem_resultado_e_outras_chamadas_nao_entram() {
        let t = b"391   read(9,  <unfinished ...>\n391   <... read resumed> 0xb588ea24, 512) = -1 EAGAIN (Try again)\n387   write(13, \"\\x55\", 1) = 1\n387   pread64(3, \"ab\", 2, 0) = 2\n";
        assert!(leituras(t).is_empty());
    }

    /// Um quadro partido entre duas leituras do mesmo descritor sai inteiro; o de outro descritor não se mistura.
    #[test]
    fn quadro_partido_entre_leituras_sai_inteiro() {
        let (q, _) = quadro_hex();
        let (a, b) = q.split_at(5);
        let hx = |s: &[u8]| s.iter().map(|x| format!("\\x{x:02x}")).collect::<String>();
        let t = format!(
            "read(13, \"{}\", 256) = 5\nread(14, \"{}\", 256) = 5\nread(13, \"{}\", 256) = 9\n",
            hx(a),
            hx(a),
            hx(b)
        );
        let qs = quadros(t.as_bytes());
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].1.montar().unwrap(), q);
    }
}
