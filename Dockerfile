FROM ubuntu:24.04 as builder

RUN apt-get update && apt install -y git libtool automake autoconf make tini ca-certificates curl

RUN git clone https://github.com/Comcast/Infinite-File-Curtailer.git curtailer \
	&& cd curtailer \
	&& libtoolize \
	&& aclocal \
	&& autoheader \
	&& autoconf \
	&& automake --add-missing \
	&& ./configure \
	&& make \
	&& make install \
	&& curtail --version

FROM ubuntu:24.04 as tools

RUN apt-get update && apt install -y git curl ca-certificates

ENV ASDF_DIR=/opt/asdf
RUN git clone https://github.com/asdf-vm/asdf.git ${ASDF_DIR} --branch v0.14.0
ENV PATH="${ASDF_DIR}/bin:${ASDF_DIR}/shims:${PATH}"

COPY .tool-versions /tmp/.tool-versions

WORKDIR /tmp
RUN . /opt/asdf/asdf.sh \
	&& asdf plugin add vrf https://github.com/cartridge-gg/vrf.git \
	&& asdf plugin add paymaster https://github.com/cartridge-gg/paymaster.git \
	&& VRF_VERSION="$(awk '$1==\"vrf\" {print $2}' /tmp/.tool-versions)" \
	&& PAYMASTER_VERSION="$(awk '$1==\"paymaster\" {print $2}' /tmp/.tool-versions)" \
	&& asdf install vrf "${VRF_VERSION}" \
	&& asdf install paymaster "${PAYMASTER_VERSION}" \
	&& cp "$(asdf which vrf-server)" /usr/local/bin/vrf-server \
	&& cp "$(asdf which paymaster-service)" /usr/local/bin/paymaster-service

FROM ubuntu:24.04 as base

# Required by cairo-native 
RUN apt-get update && apt install -y binutils clang-19

COPY --from=builder /etc/ssl/certs /etc/ssl/certs
COPY --from=builder /usr/bin/curl /usr/bin/curl

COPY --from=builder /usr/bin/tini /tini
ENTRYPOINT ["/tini", "--"]

ARG TARGETPLATFORM

LABEL description="Dojo is a provable game engine and toolchain for building onchain games and autonomous worlds with Cairo" \
	authors="Ammar Arif <evergreenkary@gmail.com>" \
	source="https://github.com/dojoengine/katana" \
	documentation="https://book.dojoengine.org/"

COPY --from=artifacts --chmod=755 $TARGETPLATFORM/katana /usr/local/bin/
COPY --from=tools --chmod=755 /usr/local/bin/paymaster-service /usr/local/bin/
COPY --from=tools --chmod=755 /usr/local/bin/vrf-server /usr/local/bin/

COPY --from=builder /usr/local/bin/curtail /usr/local/bin/curtail
