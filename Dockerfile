FROM ghcr.io/dojoengine/dojo-dev:v1.2.2

# Set environment variables for LLVM
ENV MLIR_SYS_190_PREFIX=/usr/lib/llvm-19
ENV	LLVM_SYS_191_PREFIX=/usr/lib/llvm-19
ENV TABLEGEN_190_PREFIX=/usr/lib/llvm-19

# Install LLVM and Cairo native dependencies
RUN apt-get install -y \
	llvm-19 \
	llvm-19-dev \
	llvm-19-runtime \
	clang-19 \
	clang-tools-19 \
	lld-19 \
	libpolly-19-dev \
	libmlir-19-dev \
	mlir-19-tools

ENV PYENV_ROOT="/root/.pyenv"
ENV PATH="/root/.pyenv/bin:$PATH"
RUN curl -fsSL https://pyenv.run | bash
RUN echo 'export PYENV_ROOT="/root/.pyenv"' >> /root/.bashrc && \
	echo 'export PATH="$PYENV_ROOT/bin:$PATH"' >> /root/.bashrc && \
	echo 'eval "$(pyenv init -)"' >> /root/.bashrc && \
	echo 'eval "$(pyenv virtualenv-init -)"' >> /root/.bashrc

# Add shims to PATH for non-login shell usage
ENV PATH="/root/.pyenv/shims:$PATH"
