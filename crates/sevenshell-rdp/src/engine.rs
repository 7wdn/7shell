//! The connection sequence and the loop that carries an open desktop.

use std::net::SocketAddr;
use std::time::Duration;

use ironrdp_connector::connection_activation::{ConnectionActivationFactory, ConnectionActivationState};
use ironrdp_connector::{
    ClientConnector, ClientConnectorState, ConnectionOutcome, ConnectionResult, ConnectorError, Credentials,
};
use ironrdp_core::WriteBuf;
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp_dvc::DrdynvcClient;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_input::{Database, MouseButton as RdpMouseButton, MousePosition, Operation, Scancode, WheelRotations};
use ironrdp_pdu::gcc::{ConnectionType, KeyboardType};
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_session::fast_path::ProcessorBuilder;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{
    FramedWrite as _, TokioFramed, connect_begin, connect_finalize, mark_as_upgraded, single_sequence_step_read,
    split_tokio_framed,
};
use log::{debug, info};
use sevenshell_error::ShellError;

use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::{DesktopCommand, DesktopEvent, DesktopSize, Frame, InputEvent, MouseButton, OpenRequest, Target};

type UpgradedFramed = TokioFramed<ironrdp_tls::TlsStream<TcpStream>>;

/// How long the whole connect sequence — lookup, negotiation, TLS, authentication — may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn connect(request: &OpenRequest) -> Result<(UpgradedFramed, ConnectionResult), ShellError> {
    let host = request.target.host.clone();
    let port = request.target.port;

    tokio::time::timeout(CONNECT_TIMEOUT, connect_inner(request))
        .await
        .map_err(|_| ShellError::rdp_unreachable(&host, port, "the connection step timed out"))?
}

async fn connect_inner(request: &OpenRequest) -> Result<(UpgradedFramed, ConnectionResult), ShellError> {
    let host = request.target.host.as_str();
    let port = request.target.port;

    let addr = resolve(host, port).await?;
    let stream = TcpStream::connect(addr)
        .await
        .map_err(|error| ShellError::rdp_unreachable(host, port, error.to_string()))?;
    let _ = stream.set_nodelay(true);
    let local = stream
        .local_addr()
        .map_err(|error| ShellError::rdp_unreachable(host, port, error.to_string()))?;

    let mut framed = TokioFramed::new(stream);
    let config = connector_config(&request.target, request.password.as_str(), request.desktop);
    let mut connector = ClientConnector::new(config, local).with_static_channel(dynamic_channels());

    let should_upgrade = connect_begin(&mut framed, &mut connector)
        .await
        .map_err(|error| classify(host, port, &error))?;
    require_tls(&connector, host, port)?;

    let (stream, leftover) = framed.into_inner();
    let (tls, certificate) = ironrdp_tls::upgrade(stream, host)
        .await
        .map_err(|error| ShellError::rdp_tls_failed(host, port, error.to_string()))?;

    // The decision happens here: before this point the server has seen a TLS handshake and
    // nothing else, and no credential has been offered.
    let (presented, _) = super::trust::fingerprint_parts(&certificate)
        .ok_or_else(|| ShellError::rdp_tls_failed(host, port, "the certificate could not be read"))?;
    if presented != request.expected_certificate {
        return Err(ShellError::rdp_certificate_changed(host, port));
    }

    let server_public_key = certificate
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| ShellError::rdp_tls_failed(host, port, "the server's public key is not byte-aligned"))?
        .to_owned();

    let upgraded = mark_as_upgraded(should_upgrade, &mut connector);
    let mut framed = TokioFramed::new_with_leftover(tls, leftover);
    let mut network = ReqwestNetworkClient::new();

    let result = connect_finalize(
        upgraded,
        connector,
        &mut framed,
        &mut network,
        host.to_owned().into(),
        server_public_key,
        None,
    )
    .await
    .map_err(|error| classify(host, port, &error))?;

    let ConnectionOutcome::Connected(result) = result else {
        return Err(ShellError::rdp_protocol_failed(
            host,
            port,
            "the server redirected this connection to another host, which this build does not follow",
        ));
    };

    info!(
        "remote desktop connected to {host}:{port} ({}x{})",
        result.desktop_size.width, result.desktop_size.height
    );

    Ok((framed, result))
}

/// The X.224 negotiation followed by a TLS handshake, with no authentication. Used to show a
/// fingerprint before there is anything to authenticate with.
pub(crate) async fn probe_certificate(target: &Target) -> Result<x509_cert::Certificate, ShellError> {
    let host = target.host.as_str();
    let port = target.port;

    let addr = resolve(host, port).await?;
    let stream = TcpStream::connect(addr)
        .await
        .map_err(|error| ShellError::rdp_unreachable(host, port, error.to_string()))?;
    let local = stream
        .local_addr()
        .map_err(|error| ShellError::rdp_unreachable(host, port, error.to_string()))?;

    let mut framed = TokioFramed::new(stream);
    let config = connector_config(target, "", DesktopSize::default());
    let mut connector = ClientConnector::new(config, local);

    connect_begin(&mut framed, &mut connector)
        .await
        .map_err(|error| classify(host, port, &error))?;
    require_tls(&connector, host, port)?;

    let (stream, _leftover) = framed.into_inner();
    let (_tls, certificate) = ironrdp_tls::upgrade(stream, host)
        .await
        .map_err(|error| ShellError::rdp_tls_failed(host, port, error.to_string()))?;

    Ok(certificate)
}

async fn resolve(host: &str, port: u16) -> Result<SocketAddr, ShellError> {
    tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| ShellError::rdp_unreachable(host, port, error.to_string()))?
        .next()
        .ok_or_else(|| ShellError::rdp_unreachable(host, port, "the name resolved to no address"))
}

fn classify(host: &str, port: u16, error: &ConnectorError) -> ShellError {
    let text = error.to_string();
    let lower = text.to_ascii_lowercase();

    // Connector errors do not carry a machine-readable authentication flag, so the few spellings
    // CredSSP and the server use are the distinction between "the password is wrong" and "the
    // protocol broke". The error text carries no credential, so it is safe to keep as detail.
    if lower.contains("credssp")
        || lower.contains("logon")
        || lower.contains("access denied")
        || lower.contains("authentication")
        || lower.contains("ntlm")
    {
        ShellError::rdp_auth_failed(host, port)
    } else {
        ShellError::rdp_protocol_failed(host, port, text)
    }
}

fn require_tls(connector: &ClientConnector, host: &str, port: u16) -> Result<(), ShellError> {
    if let ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol } = connector.state
        && selected_protocol.intersects(SecurityProtocol::SSL | SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX)
    {
        // The difference matters when checking a real Windows host: HYBRID/HYBRID_EX means NLA
        // (CredSSP authenticates before the session), SSL alone means the password is entered
        // inside the RDP session.
        info!("remote desktop security layer: {selected_protocol:?}");
        return Ok(());
    }

    Err(ShellError::rdp_tls_failed(
        host,
        port,
        "the server did not negotiate a TLS security layer",
    ))
}

fn connector_config(target: &Target, password: &str, desktop: DesktopSize) -> ironrdp_connector::Config {
    ironrdp_connector::Config {
        credentials: Credentials::UsernamePassword {
            username: target.user.clone(),
            password: password.to_owned(),
        },
        domain: if target.domain.is_empty() {
            None
        } else {
            Some(target.domain.clone())
        },
        enable_tls: true,
        enable_credssp: true,
        enable_standard_rdp_security: false,
        keyboard_type: KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: ironrdp_connector::DesktopSize {
            width: desktop.width,
            height: desktop.height,
        },
        monitor_layout: None,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        monitor_extended_layout: None,
        relative_mouse_input: false,
        bitmap: Some(ironrdp_connector::BitmapConfig {
            // RemoteFX is the difference between "a slideshow over the network" and a desktop:
            // without it the server falls back to legacy bitmap updates. The empty list means
            // the defaults, which include RemoteFX (and QOI where this build carries it).
            lossy_compression: true,
            color_depth: 32,
            codecs: ironrdp_pdu::rdp::capability_sets::client_codecs_capabilities(&[])
                .expect("the default bitmap codec list is valid"),
        }),
        client_build: 0,
        client_name: "7Shell".to_owned(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        connection_type: ConnectionType::Autodetect,
        platform: platform(),
        enable_server_pointer: false,
        request_data: None,
        autologon: true,
        enable_audio_playback: false,
        enable_audio_capture: false,
        compression_type: None,
        pointer_software_rendering: true,
        multitransport_flags: None,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        remote_application_mode: false,
        rail_support_level: RailSupportLevel::empty(),
        support_dyn_vc_gfx_protocol: false,
    }
}

fn platform() -> MajorPlatformType {
    #[cfg(windows)]
    {
        MajorPlatformType::WINDOWS
    }
    #[cfg(target_os = "macos")]
    {
        MajorPlatformType::MACINTOSH
    }
    #[cfg(target_os = "linux")]
    {
        MajorPlatformType::UNIX
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        MajorPlatformType::UNIX
    }
}

fn dynamic_channels() -> DrdynvcClient {
    DrdynvcClient::new().with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())))
}

pub(crate) async fn run(
    framed: UpgradedFramed,
    connection: ConnectionResult,
    mut commands: mpsc::Receiver<DesktopCommand>,
    events: mpsc::UnboundedSender<DesktopEvent>,
    host: String,
    port: u16,
) {
    match run_inner(framed, connection, &mut commands, &events, &host, port).await {
        Ok(()) => {
            let _ = events.send(DesktopEvent::Closed);
        }
        Err(error) => {
            let _ = events.send(DesktopEvent::Failed(error));
        }
    }
}

async fn run_inner(
    framed: UpgradedFramed,
    connection: ConnectionResult,
    commands: &mut mpsc::Receiver<DesktopCommand>,
    events: &mpsc::UnboundedSender<DesktopEvent>,
    host: &str,
    port: u16,
) -> Result<(), ShellError> {
    let (mut reader, mut writer) = split_tokio_framed(framed);
    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        connection.desktop_size.width,
        connection.desktop_size.height,
    );
    let activation_factory = connection.activation_factory;
    let refresh_rect_support = connection.refresh_rect_support;
    let suppress_output_support = connection.suppress_output_support;
    let mut stage = ActiveStageBuilder {
        static_channels: connection.static_channels,
        user_channel_id: connection.user_channel_id,
        io_channel_id: connection.io_channel_id,
        message_channel_id: connection.message_channel_id,
        share_id: connection.share_id,
        compression_type: connection.compression_type,
        enable_server_pointer: connection.enable_server_pointer,
        pointer_software_rendering: connection.pointer_software_rendering,
    }
    .build();
    let mut input = Database::new();

    let mut closing = false;
    'outer: loop {
        let outputs = tokio::select! {
            frame = reader.read_pdu() => {
                let (action, payload) = frame
                    .map_err(|error| ShellError::rdp_protocol_failed(host, port, error.to_string()))?;
                stage
                    .process(&mut image, action, &payload, reader.last_read_at())
                    .map_err(|error| ShellError::rdp_protocol_failed(host, port, error.to_string()))?
            }
            command = commands.recv() => {
                let Some(command) = command else { break 'outer };
                let (outputs, stop) = handle(
                    command,
                    &mut stage,
                    &mut image,
                    &mut input,
                    host,
                    port,
                    refresh_rect_support,
                    suppress_output_support,
                )?;
                closing = stop;
                outputs
            }
        };

        let mut pending: Vec<Frame> = Vec::new();
        let mut terminated = false;

        for output in outputs {
            match output {
                ActiveStageOutput::ResponseFrame(frame) => writer
                    .write_all(&frame)
                    .await
                    .map_err(|error| ShellError::rdp_protocol_failed(host, port, error.to_string()))?,
                ActiveStageOutput::GraphicsUpdate(region) => {
                    pending.push(copy_region(&image, region));
                }
                ActiveStageOutput::DeactivateAll => {
                    debug!("remote desktop asked to deactivate; reactivating");
                    let (next_image, size) =
                        reactivate(&mut reader, &mut writer, &mut stage, &activation_factory, host, port).await?;
                    image = next_image;
                    let _ = events.send(DesktopEvent::Resized {
                        width: size.0,
                        height: size.1,
                    });
                }
                ActiveStageOutput::Terminate(reason) => {
                    debug!("remote desktop terminated: {reason:?}");
                    terminated = true;
                    break;
                }
                ActiveStageOutput::PointerDefault
                | ActiveStageOutput::PointerHidden
                | ActiveStageOutput::PointerPosition { .. }
                | ActiveStageOutput::PointerBitmap(_) => {}
                ActiveStageOutput::MultitransportRequest(_) | ActiveStageOutput::AutoDetect(_) => {}
                _ => {}
            }
        }

        if !pending.is_empty() && events.send(DesktopEvent::Frames(pending)).is_err() {
            break 'outer;
        }

        if closing || terminated {
            break 'outer;
        }
    }

    Ok(())
}

fn handle(
    command: DesktopCommand,
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    input: &mut Database,
    host: &str,
    port: u16,
    refresh_rect_support: bool,
    suppress_output_support: bool,
) -> Result<(Vec<ActiveStageOutput>, bool), ShellError> {
    match command {
        DesktopCommand::Input(event) => {
            let events = input.apply([operation(event)]);
            let outputs = stage
                .process_fastpath_input(image, &events)
                .map_err(|error| ShellError::rdp_protocol_failed(host, port, error.to_string()))?;
            Ok((outputs, false))
        }
        DesktopCommand::Resize { width, height } => {
            let (width, height) = MonitorLayoutEntry::adjust_display_size(u32::from(width), u32::from(height));
            match stage.encode_resize(width, height, Some(100), None) {
                Some(Ok(frame)) => Ok((vec![ActiveStageOutput::ResponseFrame(frame)], false)),
                Some(Err(error)) => Err(ShellError::rdp_resize_failed(error.to_string())),
                None => {
                    debug!("the server did not negotiate the display-control channel; resize ignored");
                    Ok((Vec::new(), false))
                }
            }
        }
        DesktopCommand::Refresh => {
            let frames = stage
                .request_full_redraw(
                    image.width(),
                    image.height(),
                    refresh_rect_support,
                    suppress_output_support,
                )
                .map_err(|error| ShellError::rdp_protocol_failed(host, port, error.to_string()))?;
            Ok((
                frames.into_iter().map(ActiveStageOutput::ResponseFrame).collect(),
                false,
            ))
        }
        DesktopCommand::Close => {
            let outputs = stage
                .graceful_shutdown()
                .map_err(|error| ShellError::rdp_protocol_failed(host, port, error.to_string()))?;
            Ok((outputs, true))
        }
    }
}

fn operation(event: InputEvent) -> Operation {
    match event {
        InputEvent::MouseMove { x, y } => Operation::MouseMove(MousePosition { x, y }),
        InputEvent::MouseButton { button, pressed } => {
            let button = match button {
                MouseButton::Left => RdpMouseButton::Left,
                MouseButton::Right => RdpMouseButton::Right,
                MouseButton::Middle => RdpMouseButton::Middle,
                MouseButton::Back => RdpMouseButton::X1,
                MouseButton::Forward => RdpMouseButton::X2,
            };
            if pressed {
                Operation::MouseButtonPressed(button)
            } else {
                Operation::MouseButtonReleased(button)
            }
        }
        InputEvent::Wheel { vertical, units } => Operation::WheelRotations(WheelRotations {
            is_vertical: vertical,
            rotation_units: units,
        }),
        InputEvent::Key { scancode, pressed } => {
            let scancode = Scancode::from_u16(scancode);
            if pressed {
                Operation::KeyPressed(scancode)
            } else {
                Operation::KeyReleased(scancode)
            }
        }
        InputEvent::Unicode { character, pressed } => {
            if pressed {
                Operation::UnicodeKeyPressed(character)
            } else {
                Operation::UnicodeKeyReleased(character)
            }
        }
    }
}

/// Copies one changed rectangle out of the frame buffer as RGBA8.
///
/// `DecodedImage` stores rows at the desktop's stride, not at the rectangle's width; only the
/// rectangle's bytes are copied, so the frame that crosses IPC is dense.
fn copy_region(image: &DecodedImage, region: InclusiveRectangle) -> Frame {
    let width = region.right - region.left + 1;
    let height = region.bottom - region.top + 1;
    let stride = image.stride();
    let bytes_per_pixel = image.bytes_per_pixel();
    let mut rgba = Vec::with_capacity(usize::from(width) * usize::from(height) * bytes_per_pixel);

    for y in region.top..=region.bottom {
        let start = usize::from(y) * stride + usize::from(region.left) * bytes_per_pixel;
        let end = start + usize::from(width) * bytes_per_pixel;
        rgba.extend_from_slice(&image.data()[start..end]);
    }

    Frame {
        x: region.left,
        y: region.top,
        width,
        height,
        rgba,
    }
}

/// The Deactivation-Reactivation Sequence, used when the far side changes the desktop size.
async fn reactivate(
    reader: &mut TokioFramed<tokio::io::ReadHalf<ironrdp_tls::TlsStream<TcpStream>>>,
    writer: &mut TokioFramed<tokio::io::WriteHalf<ironrdp_tls::TlsStream<TcpStream>>>,
    stage: &mut ActiveStage,
    factory: &ConnectionActivationFactory,
    host: &str,
    port: u16,
) -> Result<(DecodedImage, (u16, u16)), ShellError> {
    let mut activation = factory.create();
    let mut buf = WriteBuf::new();

    loop {
        let written = single_sequence_step_read(reader, &mut activation, &mut buf)
            .await
            .map_err(|error| ShellError::rdp_protocol_failed(host, port, error.to_string()))?;
        if written.size().is_some() {
            writer
                .write_all(buf.filled())
                .await
                .map_err(|error| ShellError::rdp_protocol_failed(host, port, error.to_string()))?;
        }

        if let ConnectionActivationState::Finalized {
            desktop_size,
            share_id,
            enable_server_pointer,
            pointer_software_rendering,
            ..
        } = activation.connection_activation_state()
        {
            let image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
            stage.set_fastpath_processor(
                ProcessorBuilder {
                    io_channel_id: activation.io_channel_id(),
                    user_channel_id: activation.user_channel_id(),
                    share_id,
                    enable_server_pointer,
                    pointer_software_rendering,
                }
                .build(),
            );
            stage.set_share_id(share_id);
            stage.set_enable_server_pointer(enable_server_pointer);
            return Ok((image, (desktop_size.width, desktop_size.height)));
        }
    }
}
