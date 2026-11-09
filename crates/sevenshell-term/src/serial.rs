//! A serial port as a terminal: RS-232 consoles, routers and development boards.
//!
//! The port is a blocking device, so it lives on its own read thread with a short timeout while the
//! async side multiplexes commands. A read that returns no bytes is the normal idle state, not a
//! failure; a read error of any other kind means the device went away, which closes the session.
//!
//! Line parameters are checked before the port is opened. A wrong baud rate is indistinguishable
//! from a dead device, so silently falling back to 8N1 would send somebody hunting the wrong
//! problem; the error names the value instead.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sevenshell_error::ShellError;
use sevenshell_types::{
    BackspaceMode, SerialFlowControl, SerialParity, SerialPortInfo, SerialSettings, SerialStopBits,
};
use tokio::sync::mpsc;

use crate::TerminalEvent;

/// How long one port read may wait before returning "nothing yet".
const READ_TIMEOUT: Duration = Duration::from_millis(10);

/// What to open and how the line is framed.
#[derive(Debug, Clone)]
pub struct SerialSpec {
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub parity: SerialParity,
    pub stop_bits: SerialStopBits,
    pub flow_control: SerialFlowControl,
    pub backspace: BackspaceMode,
}

impl From<&SerialSettings> for SerialSpec {
    fn from(settings: &SerialSettings) -> Self {
        Self {
            port: settings.port.clone(),
            baud_rate: settings.baud_rate,
            data_bits: settings.data_bits,
            parity: settings.parity,
            stop_bits: settings.stop_bits,
            flow_control: settings.flow_control,
            backspace: settings.backspace,
        }
    }
}

/// A serial console that is open.
pub struct SerialSession {
    commands: mpsc::Sender<Command>,
}

enum Command {
    Input(Vec<u8>),
    Close,
}

impl SerialSession {
    pub async fn write(&self, data: &[u8]) -> Result<(), ShellError> {
        self.commands
            .send(Command::Input(data.to_vec()))
            .await
            .map_err(|_| ShellError::session_not_found("the serial port is no longer open"))
    }

    /// A serial line has no window to resize. This exists so the caller does not need to know
    /// which transport a handle belongs to.
    pub async fn resize(&self, _columns: u32, _rows: u32) -> Result<(), ShellError> {
        Ok(())
    }

    pub async fn close(&self) -> Result<(), ShellError> {
        let _ = self.commands.send(Command::Close).await;
        Ok(())
    }
}

/// The ports this machine has, sorted by device path.
pub fn serial_ports() -> Vec<SerialPortInfo> {
    let mut ports: Vec<SerialPortInfo> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|entry| SerialPortInfo {
            path: entry.port_name,
            description: description_of(&entry.port_type),
        })
        .collect();
    ports.sort_by(|left, right| left.path.cmp(&right.path));
    ports
}

fn description_of(kind: &serialport::SerialPortType) -> Option<String> {
    match kind {
        serialport::SerialPortType::UsbPort(info) => Some(
            info.product
                .clone()
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("USB {:04x}:{:04x}", info.vid, info.pid)),
        ),
        serialport::SerialPortType::PciPort => Some("PCI".to_owned()),
        serialport::SerialPortType::BluetoothPort => Some("Bluetooth".to_owned()),
        serialport::SerialPortType::Unknown => None,
    }
}

/// What the console needs from its port.
///
/// The real implementation is a `serialport` handle; a test one is a pair of channels. Keeping the
/// pump behind this trait is what lets the byte path — including the backspace rewrite, the write
/// queue's ordering and the close — be exercised without a device on the machine.
trait SerialIo: Send {
    fn read_from_device(&mut self, buffer: &mut [u8]) -> std::io::Result<usize>;
    fn write_to_device(&mut self, data: &[u8]) -> std::io::Result<()>;
}

struct RealPort(Box<dyn serialport::SerialPort>);

impl SerialIo for RealPort {
    fn read_from_device(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buffer)
    }

    fn write_to_device(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.0.write_all(data)?;
        self.0.flush()
    }
}

/// Opens the port and starts its read thread and pump.
pub async fn open(spec: SerialSpec) -> Result<(SerialSession, mpsc::Receiver<TerminalEvent>), ShellError> {
    let port_name = spec.port.trim().to_owned();
    if port_name.is_empty() {
        return Err(ShellError::terminal_invalid("a serial session needs a port"));
    }
    if !(50..=4_000_000).contains(&spec.baud_rate) {
        return Err(ShellError::terminal_invalid(format!(
            "baud rate {} is outside the driver range 50..=4000000",
            spec.baud_rate
        )));
    }
    if !(5..=8).contains(&spec.data_bits) {
        return Err(ShellError::terminal_invalid(format!(
            "{} data bits is not a serial frame (5, 6, 7 or 8)",
            spec.data_bits
        )));
    }

    let open_name = port_name.clone();
    let port = tokio::task::spawn_blocking(move || {
        serialport::new(&open_name, spec.baud_rate)
            .data_bits(data_bits(spec.data_bits))
            .parity(parity(spec.parity))
            .stop_bits(stop_bits(spec.stop_bits))
            .flow_control(flow_control(spec.flow_control))
            .timeout(READ_TIMEOUT)
            .open()
            .map(|port| Box::new(RealPort(port)) as Box<dyn SerialIo>)
            .map_err(|error| ShellError::terminal_serial_failed(&open_name, error.to_string()))
    })
    .await
    .map_err(|error| ShellError::app_internal(format!("opening the serial port panicked: {error}")))??;

    Ok(start(Arc::new(Mutex::new(port)), spec.backspace, port_name))
}

/// Starts the reader and the pump over a port that is already open.
fn start(
    port: Arc<Mutex<Box<dyn SerialIo>>>,
    backspace: BackspaceMode,
    port_name: String,
) -> (SerialSession, mpsc::Receiver<TerminalEvent>) {
    let closed = Arc::new(AtomicBool::new(false));
    let (events, receiver) = mpsc::channel(256);
    let (commands, command_receiver) = mpsc::channel(128);

    spawn_reader(Arc::clone(&port), Arc::clone(&closed), &port_name, events.clone());
    tokio::spawn(pump(port, closed, backspace, port_name, command_receiver, events));

    (SerialSession { commands }, receiver)
}

fn data_bits(bits: u8) -> serialport::DataBits {
    match bits {
        5 => serialport::DataBits::Five,
        6 => serialport::DataBits::Six,
        7 => serialport::DataBits::Seven,
        _ => serialport::DataBits::Eight,
    }
}

fn parity(value: SerialParity) -> serialport::Parity {
    match value {
        SerialParity::None => serialport::Parity::None,
        SerialParity::Even => serialport::Parity::Even,
        SerialParity::Odd => serialport::Parity::Odd,
    }
}

fn stop_bits(value: SerialStopBits) -> serialport::StopBits {
    match value {
        SerialStopBits::One => serialport::StopBits::One,
        SerialStopBits::Two => serialport::StopBits::Two,
    }
}

fn flow_control(value: SerialFlowControl) -> serialport::FlowControl {
    match value {
        SerialFlowControl::None => serialport::FlowControl::None,
        SerialFlowControl::RtsCts => serialport::FlowControl::Hardware,
        SerialFlowControl::XonXoff => serialport::FlowControl::Software,
    }
}

/// Reads the port on a thread of its own. The short read timeout is what makes closing possible
/// without a second handle: every ten milliseconds the loop sees whether it should stop.
fn spawn_reader(
    port: Arc<Mutex<Box<dyn SerialIo>>>,
    closed: Arc<AtomicBool>,
    port_name: &str,
    events: mpsc::Sender<TerminalEvent>,
) {
    let port_name = port_name.to_owned();
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            if closed.load(Ordering::Relaxed) {
                return;
            }
            let read = {
                let mut port = match port.lock() {
                    Ok(port) => port,
                    Err(poisoned) => poisoned.into_inner(),
                };
                port.read_from_device(&mut buffer)
            };
            match read {
                Ok(0) => continue,
                Ok(count) => {
                    if events
                        .blocking_send(TerminalEvent::Data(buffer[..count].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    let _ = events.blocking_send(TerminalEvent::Closed {
                        reason: Some(format!("serial port {port_name} stopped answering: {error}")),
                    });
                    return;
                }
            }
        }
    });
}

async fn pump(
    port: Arc<Mutex<Box<dyn SerialIo>>>,
    closed: Arc<AtomicBool>,
    backspace: BackspaceMode,
    port_name: String,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<TerminalEvent>,
) {
    let mut reason = None;
    while let Some(command) = commands.recv().await {
        match command {
            Command::Input(data) => {
                let wire = rewrite_backspace(&data, backspace);
                if wire.is_empty() {
                    continue;
                }
                let port = Arc::clone(&port);
                let written = tokio::task::spawn_blocking(move || {
                    let mut port = match port.lock() {
                        Ok(port) => port,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    port.write_to_device(&wire)
                })
                .await;
                match written {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        reason = Some(format!("serial port {port_name} stopped accepting input: {error}"));
                        break;
                    }
                    Err(error) => {
                        reason = Some(format!("the serial write task ended: {error}"));
                        break;
                    }
                }
            }
            Command::Close => break,
        }
    }
    closed.store(true, Ordering::Relaxed);
    let _ = events.send(TerminalEvent::Closed { reason }).await;
}

fn rewrite_backspace(data: &[u8], mode: BackspaceMode) -> Vec<u8> {
    match mode {
        BackspaceMode::Del => data.to_vec(),
        BackspaceMode::CtrlH => data
            .iter()
            .map(|byte| if *byte == 0x7f { 0x08 } else { *byte })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
    use std::time::{Duration, Instant};

    use super::*;

    /// A port that is two channels: bytes "the device sends" in, bytes the host writes out.
    struct FakePort {
        incoming: Receiver<Vec<u8>>,
        outgoing: Sender<Vec<u8>>,
        pending: Vec<u8>,
        offset: usize,
    }

    impl SerialIo for FakePort {
        fn read_from_device(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.offset >= self.pending.len() {
                match self.incoming.try_recv() {
                    Ok(chunk) => {
                        self.pending = chunk;
                        self.offset = 0;
                    }
                    Err(TryRecvError::Empty) => {
                        // The real port blocks for its read timeout; sleeping is what keeps this
                        // fake from spinning while the pump is idle.
                        std::thread::sleep(Duration::from_millis(2));
                        return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "idle"));
                    }
                    Err(TryRecvError::Disconnected) => {
                        return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "device gone"));
                    }
                }
            }
            let count = (self.pending.len() - self.offset).min(buffer.len());
            buffer[..count].copy_from_slice(&self.pending[self.offset..self.offset + count]);
            self.offset += count;
            Ok(count)
        }

        fn write_to_device(&mut self, data: &[u8]) -> std::io::Result<()> {
            self.outgoing
                .send(data.to_vec())
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "host gone"))
        }
    }

    async fn next_event(events: &mut tokio::sync::mpsc::Receiver<TerminalEvent>) -> TerminalEvent {
        tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("an event within five seconds")
            .expect("the channel is still open")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_serial_console_carries_bytes_both_ways_and_closes() {
        let (device, incoming) = channel();
        let (outgoing, host) = channel();
        let port: Arc<Mutex<Box<dyn SerialIo>>> = Arc::new(Mutex::new(Box::new(FakePort {
            incoming,
            outgoing,
            pending: Vec::new(),
            offset: 0,
        })));
        let (session, mut events) = start(port, BackspaceMode::CtrlH, "fake".to_owned());

        device.send(b"login: ".to_vec()).expect("the device speaks");
        match next_event(&mut events).await {
            TerminalEvent::Data(bytes) => assert_eq!(bytes, b"login: "),
            TerminalEvent::Closed { reason } => panic!("the console closed early: {reason:?}"),
        }

        // Ctrl+H mode rewrites DEL on the way out; the bytes reaching the device are the proof.
        session.write(b"a\x7fb").await.expect("a write");
        let written = host
            .recv_timeout(Duration::from_secs(5))
            .expect("the device sees the bytes");
        assert_eq!(written, b"a\x08b");

        // A serial line has no window, but the caller does not know that and neither should fail.
        session.resize(120, 40).await.expect("resize is a no-op here");

        session.close().await.expect("a close");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match next_event(&mut events).await {
                TerminalEvent::Closed { reason } => {
                    assert_eq!(reason, None);
                    break;
                }
                TerminalEvent::Data(_) if Instant::now() < deadline => continue,
                TerminalEvent::Data(_) => panic!("the console kept talking after a close"),
            }
        }
    }

    #[test]
    fn a_rate_outside_the_driver_range_is_refused_by_name() {
        let error = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(open(SerialSpec {
                port: "COM1".to_owned(),
                baud_rate: 7,
                data_bits: 8,
                parity: SerialParity::None,
                stop_bits: SerialStopBits::One,
                flow_control: SerialFlowControl::None,
                backspace: BackspaceMode::Del,
            }))
            .err()
            .expect("a nonsense baud rate cannot open a port");
        assert!(error.detail().unwrap_or_default().contains("baud rate 7"), "{error:?}");
    }

    #[test]
    fn a_frame_outside_five_to_eight_bits_is_refused() {
        let error = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(open(SerialSpec {
                port: "COM1".to_owned(),
                baud_rate: 115_200,
                data_bits: 9,
                parity: SerialParity::None,
                stop_bits: SerialStopBits::One,
                flow_control: SerialFlowControl::None,
                backspace: BackspaceMode::Del,
            }))
            .err()
            .expect("nine data bits is not a serial frame");
        assert!(error.detail().unwrap_or_default().contains("9 data bits"), "{error:?}");
    }

    #[test]
    fn ctrl_h_rewrites_only_the_delete_byte() {
        assert_eq!(rewrite_backspace(b"a\x7fb", BackspaceMode::CtrlH), b"a\x08b");
        assert_eq!(rewrite_backspace(b"a\x7fb", BackspaceMode::Del), b"a\x7fb");
    }
}
