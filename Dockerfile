ARG ARCH=amd64v8

FROM $ARCH/ubuntu:jammy AS base
RUN apt update && apt install -y --no-install-recommends \
    libgstreamer1.0-0 libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libgstreamer-plugins-good1.0-dev \
    libgstreamer-plugins-good1.0-0 gstreamer1.0-plugins-base-apps gstreamer1.0-plugins-good  \
    libssl-dev libffi-dev libgtk-3-dev libglib2.0-dev openssl && \
    rm -rf /var/lib/apt/lists/*

FROM base AS builder-base
RUN apt update && apt install -y --no-install-recommends ca-certificates pkg-config git gcc g++ make openssl unzip \
    libevent-dev libffi-dev libunwind-dev curl  && \
    rm -rf /var/lib/apt/lists/*
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --default-toolchain stable -y

FROM builder-base AS builder-arm64v8
RUN curl -Lo protoc.zip https://github.com/protocolbuffers/protobuf/releases/download/v21.0/protoc-21.0-linux-aarch_64.zip &&  \
    unzip protoc.zip -d /usr/local && chmod a+x /usr/local/bin/protoc && rm protoc.zip
WORKDIR /usr/src/gst-livekit
COPY . .
ENV PKG_CONFIG_PATH=/usr/lib/aarch64-linux-gnu/pkgconfig
RUN $HOME/.cargo/bin/cargo install --path .

FROM builder-base AS builder-amd64
RUN curl -Lo protoc.zip https://github.com/protocolbuffers/protobuf/releases/download/v21.0/protoc-21.0-linux-x86_64.zip &&  \
    unzip protoc.zip -d /usr/local && chmod a+x /usr/local/bin/protoc && rm protoc.zip
WORKDIR /usr/src/gst-livekit
COPY . .
ENV PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig
RUN $HOME/.cargo/bin/cargo install --path .

FROM $ARCH/ubuntu:jammy  as gst-livekit-base

FROM base  as gst-livekit-arm64v8
COPY --from=builder-arm64v8 /root/.cargo/bin/gstreamer-livekit /usr/local/bin/gstreamer-livekit
ENV GST_PLUGIN_SCANNER=/usr/lib/aarch64-linux-gnu/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner

FROM base  as gst-livekit-amd64
COPY --from=builder-amd64 /root/.cargo/bin/gstreamer-livekit /usr/local/bin/gstreamer-livekit
ENV GST_PLUGIN_SCANNER=/usr/lib/x86_64-linux-gnu/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner

FROM gst-livekit-${ARCH} as gst-livekit
CMD ["gstreamer-livekit"]