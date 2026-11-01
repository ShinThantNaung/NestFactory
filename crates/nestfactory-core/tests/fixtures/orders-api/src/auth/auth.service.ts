import { Injectable } from '@nestjs/common';
import { ConfigService } from '@nestjs/config';
import { TokenService } from './token.service';

@Injectable()
export class AuthService {
  constructor(
    private readonly tokens: TokenService,
    private readonly config: ConfigService,
  ) {}
}
