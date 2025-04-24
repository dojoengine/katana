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

COPY --from=builder /usr/local/bin/curtail /usr/local/bin/curtail
