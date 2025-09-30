FROM node:20-alpine
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev
COPY server.js ./
ENV PORT=8080
ENV DEMO_MANIFEST_DIR=/manifests
EXPOSE 8080
CMD ["node", "server.js"]
