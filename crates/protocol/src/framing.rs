/// 长度前缀（LengthDelimited）帧的最大字节数。
///
/// 默认的 `tokio_util::codec::LengthDelimitedCodec` 上限偏小，控制通道在发送包含历史输出的
/// `ServiceSnapshot` 时可能超过限制，从而导致客户端报 `read control frame` 一类的解码错误。
///
/// 该上限需要在客户端与服务端保持一致。
pub const MAX_FRAME_LENGTH: usize = 256 * 1024 * 1024;
