//! O corpo atrás da mesma interface: serial no robô, rede no simulador, falso nos testes. Só o tipo em
//! `rms1d.toml` muda entre eles.

use std::io::{self, ErrorKind, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rms1_proto::Quadro;

pub trait Corpo: Send {
    /// Um quadro inteiro. Nunca espera mais que uma fração do ciclo.
    fn enviar(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// O que o corpo mandou desde a última chamada; zero quando nada. Não pode travar o ciclo.
    fn ler(&mut self, buf: &mut [u8]) -> io::Result<usize>;
}

impl Corpo for Box<dyn Corpo> {
    fn enviar(&mut self, bytes: &[u8]) -> io::Result<()> {
        (**self).enviar(bytes)
    }
    fn ler(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (**self).ler(buf)
    }
}

/// O robô: a UART entre o computador e o controlador de movimento.
pub struct CorpoSerial {
    porta: Box<dyn serialport::SerialPort>,
}

/// Um décimo de milissegundo de folga em relação a um ciclo de 20 ms.
const ESPERA_LEITURA: Duration = Duration::from_millis(1);

impl CorpoSerial {
    pub fn abrir(caminho: &str, baud: u32) -> io::Result<Self> {
        let porta = serialport::new(caminho, baud)
            .timeout(ESPERA_LEITURA)
            .open()
            .map_err(io::Error::other)?;
        Ok(Self { porta })
    }
}

impl Corpo for CorpoSerial {
    fn enviar(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.porta.write_all(bytes)
    }

    fn ler(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.porta.bytes_to_read().map_err(io::Error::other)? == 0 {
            return Ok(0);
        }
        match self.porta.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == ErrorKind::TimedOut => Ok(0),
            Err(e) => Err(e),
        }
    }
}

/// O simulador: os mesmos bytes, por TCP até a ponte. Sem ponte, o envio falha e o ciclo segue; a
/// reconexão é tentada uma vez por segundo.
pub struct CorpoRede {
    endereco: String,
    conexao: Option<TcpStream>,
    ultima_tentativa: Option<Instant>,
}

const INTERVALO_RECONEXAO: Duration = Duration::from_secs(1);
const ESPERA_CONEXAO: Duration = Duration::from_millis(5);

impl CorpoRede {
    pub fn new(endereco: impl Into<String>) -> Self {
        Self {
            endereco: endereco.into(),
            conexao: None,
            ultima_tentativa: None,
        }
    }

    fn conectar(&mut self) -> io::Result<&mut TcpStream> {
        if self.conexao.is_none() {
            let agora = Instant::now();
            if self
                .ultima_tentativa
                .is_some_and(|t| agora.duration_since(t) < INTERVALO_RECONEXAO)
            {
                return Err(io::Error::new(ErrorKind::NotConnected, "sem ponte"));
            }
            self.ultima_tentativa = Some(agora);
            let alvo = self.endereco.parse().map_err(io::Error::other)?;
            let s = TcpStream::connect_timeout(&alvo, ESPERA_CONEXAO)?;
            s.set_nodelay(true)?;
            s.set_nonblocking(true)?;
            tracing::info!(endereco = %self.endereco, "corpo simulado conectado");
            self.conexao = Some(s);
        }
        Ok(self.conexao.as_mut().expect("acabou de ser posta"))
    }
}

impl Corpo for CorpoRede {
    fn enviar(&mut self, bytes: &[u8]) -> io::Result<()> {
        let s = self.conectar()?;
        match s.write(bytes) {
            Ok(n) if n == bytes.len() => Ok(()),
            // Um quadro pela metade quebraria todos os seguintes; a ponte está atrás, e cair é o certo.
            Ok(_) => {
                self.conexao = None;
                Err(io::Error::new(ErrorKind::WriteZero, "quadro cortado"))
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => Err(e),
            Err(e) => {
                self.conexao = None;
                Err(e)
            }
        }
    }

    fn ler(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Desarmado, só a leitura mantém a conexão: é o corpo simulado que começa a telemetria, como o robô.
        if self.conexao.is_none() && self.conectar().is_err() {
            return Ok(0);
        }
        let Some(s) = self.conexao.as_mut() else {
            return Ok(0);
        };
        match s.read(buf) {
            Ok(0) => {
                self.conexao = None;
                Ok(0)
            }
            Ok(n) => Ok(n),
            Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(0),
            Err(e) => {
                self.conexao = None;
                Err(e)
            }
        }
    }
}

/// O corpo dos testes: guarda cada quadro enviado, e pode falhar.
#[derive(Clone, Default)]
pub struct CorpoFalso {
    pub enviados: Arc<Mutex<Vec<Quadro>>>,
    pub falhar: Arc<std::sync::atomic::AtomicBool>,
    pub a_ler: Arc<Mutex<Vec<u8>>>,
}

impl CorpoFalso {
    pub fn quadros(&self) -> Vec<Quadro> {
        self.enviados.lock().expect("trava de teste").clone()
    }

    pub fn limpar(&self) {
        self.enviados.lock().expect("trava de teste").clear();
    }
}

impl Corpo for CorpoFalso {
    fn enviar(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.falhar.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(io::Error::other("falha plantada"));
        }
        let q = Quadro::ler(bytes).map_err(io::Error::other)?;
        self.enviados.lock().expect("trava de teste").push(q);
        Ok(())
    }

    fn ler(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut a = self.a_ler.lock().expect("trava de teste");
        let n = a.len().min(buf.len());
        buf[..n].copy_from_slice(&a[..n]);
        a.drain(..n);
        Ok(n)
    }
}
