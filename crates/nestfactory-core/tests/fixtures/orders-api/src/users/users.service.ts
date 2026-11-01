import { Injectable } from '@nestjs/common';

@Injectable()
export class UsersService {
  findOne(id: number) {
    return { id, tier: 'vip' };
  }
}
