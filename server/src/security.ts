import { createHmac, randomBytes, randomUUID } from 'node:crypto';
import { jwtVerify, SignJWT } from 'jose';
import type { Role } from '@bbt/protocol';
import type { Config } from './config.js';

export interface AccessClaims {
  userId: string;
  role: Role;
  sessionId: string;
}

export function opaqueToken(bytes = 32): string {
  return randomBytes(bytes).toString('base64url');
}
export function id(): string {
  return randomUUID();
}
export function hashSecret(secret: string, value: string): string {
  return createHmac('sha256', secret).update(value).digest('hex');
}
export function createInviteCode(): string {
  const alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789';
  const bytes = randomBytes(12);
  let value = 'BBT-';
  for (let i = 0; i < 12; i += 1) {
    value += alphabet[bytes[i]! % alphabet.length];
    if (i === 3 || i === 7) value += '-';
  }
  return value;
}
export function createLobbyCode(): string {
  return opaqueToken(6).replace(/[-_]/g, 'X').slice(0, 6).toUpperCase();
}

export async function signAccessToken(config: Config, claims: AccessClaims): Promise<string> {
  return new SignJWT({ role: claims.role, sid: claims.sessionId })
    .setProtectedHeader({ alg: 'HS256' })
    .setSubject(claims.userId)
    .setIssuer(config.publicUrl)
    .setAudience('beatblock-together')
    .setIssuedAt()
    .setExpirationTime(`${config.accessTokenTtlSeconds}s`)
    .sign(new TextEncoder().encode(config.tokenSecret));
}

export async function verifyAccessToken(config: Config, token: string): Promise<AccessClaims> {
  const { payload } = await jwtVerify(token, new TextEncoder().encode(config.tokenSecret), {
    issuer: config.publicUrl,
    audience: 'beatblock-together',
  });
  if (!payload.sub || typeof payload.role !== 'string' || typeof payload.sid !== 'string')
    throw new Error('Malformed access token');
  return { userId: payload.sub, role: payload.role as Role, sessionId: payload.sid };
}
