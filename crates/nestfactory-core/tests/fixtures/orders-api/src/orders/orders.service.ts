import { Injectable } from '@nestjs/common';
import { InjectRepository } from '@nestjs/typeorm';
import { Repository } from 'typeorm';
import { UsersService } from '../users/users.service';
import { CreateOrderDto } from './dto/create-order.dto';
import { UpdateOrderDto } from './dto/update-order.dto';
import { Order } from './entities/order.entity';

@Injectable()
export class OrdersService {
  constructor(
    @InjectRepository(Order) private readonly orders: Repository<Order>,
    private readonly usersService: UsersService,
  ) {}

  async create(dto: CreateOrderDto) {
    const user = this.usersService.findOne(dto.userId);
    return this.orders.save({ ...dto, userId: user.id });
  }

  findAll() {
    return this.orders.find();
  }

  findOne(id: number) {
    return this.orders.findOneBy({ id });
  }

  update(id: number, dto: UpdateOrderDto) {
    return this.orders.update(id, dto);
  }

  async remove(id: number) {
    await this.orders.delete(id);
  }
}
