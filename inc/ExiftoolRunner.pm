package ExiftoolRunner;

use strict;
use parent 'ProcessPipe';

sub new {
  my ($class,
      $filename,
      $group,
      $start_callback,
      $successful_update_callback,
      $finished_callback,
      $group_finished_callback
     )
    = @_;

  my $self = $class->SUPER::new("exiftool -csv $filename 2>&1", $group,
				$start_callback,
				$successful_update_callback,
				$finished_callback,
				$group_finished_callback);

  $self->{csv} = Text::CSV->new ({ binary => 1, auto_diag => 1 });

  return bless $self, $class;
}

sub read_line($) {
 my ($self) = @_;

 unless(exists $self->{row1}) {
   $self->{row1} = $self->{csv}->getline($self->{shell_input});
       my $successful_update_callback = $self->{successful_update_callback};
       &$successful_update_callback($self) if defined $successful_update_callback;
   return;
 }
 unless(exists $self->{row2}) {
   $self->{row2} = $self->{csv}->getline($self->{shell_input});
   $self->finish();
       my $successful_update_callback = $self->{successful_update_callback};
       &$successful_update_callback($self) if defined $successful_update_callback;
   return;
 }
}

sub finish($) {
  my ($self) = @_;

  $self->SUPER::finish();

  $self->{exif_data} = {};

  for(my $i = 0 ; $i < scalar @{$self->{row1}} ; $i++) {
    $self->{exif_data}{$self->{row1}[$i]} = $self->{row2}[$i];
  }
}

# this adds tallies for a new exif map to the totals
sub process_exif() {
  my ($self, $total_exif_map, $new_exif_map) = @_;
  foreach my $key (keys %$new_exif_map) {
    next if($key eq 'Thumbnail Image');	# skip binary data
    next if($key eq 'Preview Image');
    next if($key eq 'Tiff Metering Image');
    my $new_value = $new_exif_map->{$key};
    if(exists $total_exif_map->{$key}{$new_value}) {
      $total_exif_map->{$key}{$new_value}++;
    } else {
      $total_exif_map->{$key}{$new_value} = 1;
    }
  }
}

1;
