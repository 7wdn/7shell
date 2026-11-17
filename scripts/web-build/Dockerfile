FROM debian:10

ENV DEBIAN_FRONTEND=noninteractive

# Buster moved to the archive; the image's sources still point at the live hosts.
RUN printf 'deb http://archive.debian.org/debian buster main\ndeb http://archive.debian.org/debian-security buster/updates main\n' > /etc/apt/sources.list \
 && printf 'Acquire::Check-Valid-Until "false";\n' > /etc/apt/apt.conf.d/99no-check-valid-until \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
      build-essential pkg-config libssl-dev libudev-dev zlib1g-dev liblzma-dev libbz2-dev \
      libfontconfig1-dev \
      curl ca-certificates git \
 && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain 1.98.1 --profile minimal

ENV PATH=/root/.cargo/bin:$PATH

WORKDIR /src
CMD ["cargo", "build", "-p", "sevenshell-web", "--release"]