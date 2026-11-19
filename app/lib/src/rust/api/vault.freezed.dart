// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'vault.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$TaskEvent {

 Object get field0;



@override
bool operator ==(Object other) {
  final _this = this as TaskEvent;
  return identical(this, other) || (other.runtimeType == runtimeType&&other is TaskEvent&&const DeepCollectionEquality().equals(other.field0, _this.field0));
}


@override
int get hashCode {
  final _this = this as TaskEvent;
  return Object.hash(runtimeType,const DeepCollectionEquality().hash(_this.field0));
}

@override
String toString() {
  final _this = this as TaskEvent;
  return 'TaskEvent(field0: ${_this.field0})';
}


}

/// @nodoc
class $TaskEventCopyWith<$Res>  {
$TaskEventCopyWith(TaskEvent _, $Res Function(TaskEvent) __);
}


/// Adds pattern-matching-related methods to [TaskEvent].
extension TaskEventPatterns on TaskEvent {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( TaskEvent_Progress value)?  progress,TResult Function( TaskEvent_Done value)?  done,required TResult orElse(),}){
final _that = this;
switch (_that) {
case TaskEvent_Progress() when progress != null:
return progress(_that);case TaskEvent_Done() when done != null:
return done(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( TaskEvent_Progress value)  progress,required TResult Function( TaskEvent_Done value)  done,}){
final _that = this;
switch (_that) {
case TaskEvent_Progress():
return progress(_that);case TaskEvent_Done():
return done(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( TaskEvent_Progress value)?  progress,TResult? Function( TaskEvent_Done value)?  done,}){
final _that = this;
switch (_that) {
case TaskEvent_Progress() when progress != null:
return progress(_that);case TaskEvent_Done() when done != null:
return done(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( ProgressInfo field0)?  progress,TResult Function( OutcomeInfo field0)?  done,required TResult orElse(),}) {final _that = this;
switch (_that) {
case TaskEvent_Progress() when progress != null:
return progress(_that.field0);case TaskEvent_Done() when done != null:
return done(_that.field0);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( ProgressInfo field0)  progress,required TResult Function( OutcomeInfo field0)  done,}) {final _that = this;
switch (_that) {
case TaskEvent_Progress():
return progress(_that.field0);case TaskEvent_Done():
return done(_that.field0);}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( ProgressInfo field0)?  progress,TResult? Function( OutcomeInfo field0)?  done,}) {final _that = this;
switch (_that) {
case TaskEvent_Progress() when progress != null:
return progress(_that.field0);case TaskEvent_Done() when done != null:
return done(_that.field0);case _:
  return null;

}
}

}

/// @nodoc


class TaskEvent_Progress extends TaskEvent {
  const TaskEvent_Progress(this.field0): super._();
  

@override final  ProgressInfo field0;

/// Create a copy of TaskEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$TaskEvent_ProgressCopyWith<TaskEvent_Progress> get copyWith => _$TaskEvent_ProgressCopyWithImpl<TaskEvent_Progress>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is TaskEvent_Progress&&(identical(other.field0, field0) || other.field0 == field0));
}


@override
int get hashCode {
    return Object.hash(runtimeType,field0);
}

@override
String toString() {
    return 'TaskEvent.progress(field0: $field0)';
}


}

/// @nodoc
abstract mixin class $TaskEvent_ProgressCopyWith<$Res> implements $TaskEventCopyWith<$Res> {
  factory $TaskEvent_ProgressCopyWith(TaskEvent_Progress value, $Res Function(TaskEvent_Progress) _then) = _$TaskEvent_ProgressCopyWithImpl;
@useResult
$Res call({
 ProgressInfo field0
});




}
/// @nodoc
class _$TaskEvent_ProgressCopyWithImpl<$Res>
    implements $TaskEvent_ProgressCopyWith<$Res> {
  _$TaskEvent_ProgressCopyWithImpl(this._self, this._then);

  final TaskEvent_Progress _self;
  final $Res Function(TaskEvent_Progress) _then;

/// Create a copy of TaskEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? field0 = null,}) {
  return _then(TaskEvent_Progress(
null == field0 ? _self.field0 : field0 // ignore: cast_nullable_to_non_nullable
as ProgressInfo,
  ));
}


}

/// @nodoc


class TaskEvent_Done extends TaskEvent {
  const TaskEvent_Done(this.field0): super._();
  

@override final  OutcomeInfo field0;

/// Create a copy of TaskEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$TaskEvent_DoneCopyWith<TaskEvent_Done> get copyWith => _$TaskEvent_DoneCopyWithImpl<TaskEvent_Done>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is TaskEvent_Done&&(identical(other.field0, field0) || other.field0 == field0));
}


@override
int get hashCode {
    return Object.hash(runtimeType,field0);
}

@override
String toString() {
    return 'TaskEvent.done(field0: $field0)';
}


}

/// @nodoc
abstract mixin class $TaskEvent_DoneCopyWith<$Res> implements $TaskEventCopyWith<$Res> {
  factory $TaskEvent_DoneCopyWith(TaskEvent_Done value, $Res Function(TaskEvent_Done) _then) = _$TaskEvent_DoneCopyWithImpl;
@useResult
$Res call({
 OutcomeInfo field0
});




}
/// @nodoc
class _$TaskEvent_DoneCopyWithImpl<$Res>
    implements $TaskEvent_DoneCopyWith<$Res> {
  _$TaskEvent_DoneCopyWithImpl(this._self, this._then);

  final TaskEvent_Done _self;
  final $Res Function(TaskEvent_Done) _then;

/// Create a copy of TaskEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? field0 = null,}) {
  return _then(TaskEvent_Done(
null == field0 ? _self.field0 : field0 // ignore: cast_nullable_to_non_nullable
as OutcomeInfo,
  ));
}


}

// dart format on
