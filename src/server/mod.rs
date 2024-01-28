use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{body, Request, Response, Version};
use pin_project_lite::pin_project;
use std::io::Error;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;
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
    pub async fn start(&self) -> Result<(), std::io::Error> {
        // Check if the address already binded,
        // if it is just return Ok(), so other test can reuse the mocked server.
        let listener = TcpListener::bind(self.addr).await;
        match listener {
            // Drop it so we can bind again later
            Ok(_) => drop(listener),
            Err(err) => match err.kind() {
                std::io::ErrorKind::AddrInUse => {
                    println!("Server alrdy exist...");
                    return Ok(());
                }
                _ => return Err(err),
            },
        }

        // Create a tcp listener and a channel.
        let (sender, mut recv) = tokio::sync::oneshot::channel::<SocketAddr>();

        // Create runtime.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let mut addr = self.addr;
        thread::spawn(move || {
            LocalSet::new()
                .block_on(&runtime, bind_server(addr, sender))
                .unwrap()
        });

        // Need to buffer for 1 second, so that the sender can send to the channel in `bind_server` function.
        // Not ideal, just a temporary hack.
        thread::sleep(Duration::from_secs(1));

        addr = recv
            .try_recv()
            .map_err(|err| Error::new(std::io::ErrorKind::NotConnected, err))?;

        println!("Successfull listening on port: {}", addr);
        Ok(())
    }
}
async fn bind_server(addr: SocketAddr, sender: Sender<SocketAddr>) -> Result<(), std::io::Error> {
    println!("Binding server...");

    let listener = TcpListener::bind(addr).await?;
    sender.send(addr).unwrap();

    // Create a service.
    let service = service_fn(|req: Request<body::Incoming>| async move {
        if req.version() == Version::HTTP_11 {
            println!("Received request");
            Ok(Response::new(Full::<Bytes>::from("hello world")))
        } else {
            println!("Error receving request");
            Err("abort")
        }
    });

    while let Ok((stream, _)) = listener.accept().await {
        spawn_local(async move {
            let io = TokioIo::new(stream);
            let _ = http1::Builder::new().serve_connection(io, service).await;
        });
    }

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
