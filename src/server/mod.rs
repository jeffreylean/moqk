use bytes::Bytes;
use http_body_util::Full;
use hyper::rt::{Read, Write};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{body, Request, Response, Version};
use log;
use pin_project_lite::pin_project;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread;
use tokio::net::TcpListener;
use tokio::sync::oneshot::Sender;
use tokio::task::{spawn_local, LocalSet};

pub struct Server {
    addr: SocketAddr,
}

pub fn new_with_port(port: u16) -> Server {
    Server {
        addr: SocketAddr::from(([127, 0, 0, 1], port)),
    }
}

pin_project! {
    pub struct TokioIo<T> {
        #[pin]
        inner: T,
    }
}

impl<T> TokioIo<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn inner(self) -> T {
        self.inner
    }
}

impl<T> hyper::rt::Read for TokioIo<T>
where
    T: tokio::io::AsyncRead,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let n = unsafe {
            let mut tbuf = tokio::io::ReadBuf::uninit(buf.as_mut());
            match tokio::io::AsyncRead::poll_read(self.project().inner, cx, &mut tbuf) {
                Poll::Ready(Ok(())) => tbuf.filled().len(),
                other => return other,
            }
        };

        unsafe {
            buf.advance(n);
        }

        Poll::Ready(Ok(()))
    }
}

impl<T> hyper::rt::Write for TokioIo<T>
where
    T: tokio::io::AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        tokio::io::AsyncWrite::poll_write(self.project().inner, cx, buf)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        tokio::io::AsyncWrite::poll_shutdown(self.project().inner, cx)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        tokio::io::AsyncWrite::poll_flush(self.project().inner, cx)
    }

    fn is_write_vectored(&self) -> bool {
        tokio::io::AsyncWrite::is_write_vectored(&self.inner)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        tokio::io::AsyncWrite::poll_write_vectored(self.project().inner, cx, bufs)
    }
}

impl Server {
    async fn start(&self) -> Result<(), std::io::Error> {
        // Create a tcp listener and a channel.
        let (sender, recv) = tokio::sync::oneshot::channel::<SocketAddr>();

        // Create runtime.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let addr = self.addr;

        thread::spawn(move || LocalSet::new().block_on(&runtime, bind_server(addr, sender)));

        Ok(())
    }
}
async fn bind_server(addr: SocketAddr, sender: Sender<SocketAddr>) -> Result<(), std::io::Error> {
    let listener = TcpListener::bind(addr).await?;
    sender.send(addr).unwrap();

    // Create a service.
    let service = service_fn(|req: Request<body::Incoming>| async move {
        if req.version() == Version::HTTP_11 {
            log::info!("Received request");
            Ok(Response::new(Full::<Bytes>::from("hello world")))
        } else {
            log::info!("Error receving request");
            Err("abort")
        }
    });

    spawn_local(async move {
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                let io = TokioIo::new(stream);
                let _ = http1::Builder::new().serve_connection(io, service);
            }
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server() -> Result<(), std::io::Error> {
        let s = new_with_port(9000);
        s.start().await
    }
}
